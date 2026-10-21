//! VideoToolbox, the hardware encoder of a Mac, called through its C
//! interface. A session is created for one size; pictures are copied into
//! pixel buffers the session's pool gives, coded behind the call, and their
//! Annex B bytes come back through the session's callback into a queue that
//! [`crate::Encoder::encode`] drains.

use std::{
    collections::VecDeque,
    ffi::c_void,
    ptr,
    sync::{Arc, Mutex},
};

use crate::{Error, Picture, PixelFormat, Raw, Rate, Result, Settings};

type CFTypeRef = *const c_void;
type CFStringRef = *const c_void;
type OSStatus = i32;

const NO_ERROR: OSStatus = 0;
const K_CF_NUMBER_SINT32: i32 = 3;
const K_CF_NUMBER_FLOAT32: i32 = 5;
/// `kCMVideoCodecType_H264`: 'avc1'.
const CODEC_H264: u32 = 0x6176_6331;
/// `kCVPixelFormatType_32BGRA`: 'BGRA'.
const PIXEL_BGRA: i32 = 0x4247_5241;
/// A pixel buffer lock that only writes.
const LOCK_WRITE: u64 = 0;
/// What `CVPixelBufferPool` answers when the pool has no more buffers: the
/// encoder holds them all.
const POOL_EXHAUSTED: i32 = -6661;

#[repr(C)]
#[derive(Clone, Copy)]
struct CMTime {
    value: i64,
    timescale: i32,
    flags: u32,
    epoch: i64,
}

impl CMTime {
    const VALID: u32 = 1;

    fn microseconds(value: u64) -> Self {
        Self {
            value: i64::try_from(value).unwrap_or(i64::MAX),
            timescale: 1_000_000,
            flags: Self::VALID,
            epoch: 0,
        }
    }

    const INVALID: Self = Self {
        value: 0,
        timescale: 0,
        flags: 0,
        epoch: 0,
    };
}

#[repr(C)]
struct CFDictionaryKeyCallBacks {
    _private: [usize; 6],
}

#[repr(C)]
struct CFDictionaryValueCallBacks {
    _private: [usize; 5],
}

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    static kCFBooleanTrue: CFTypeRef;
    static kCFBooleanFalse: CFTypeRef;
    static kCFTypeDictionaryKeyCallBacks: CFDictionaryKeyCallBacks;
    static kCFTypeDictionaryValueCallBacks: CFDictionaryValueCallBacks;
    fn CFRelease(value: CFTypeRef);
    fn CFNumberCreate(allocator: CFTypeRef, kind: i32, value: *const c_void) -> CFTypeRef;
    fn CFDictionaryCreate(
        allocator: CFTypeRef,
        keys: *const CFTypeRef,
        values: *const CFTypeRef,
        count: isize,
        key_callbacks: *const CFDictionaryKeyCallBacks,
        value_callbacks: *const CFDictionaryValueCallBacks,
    ) -> CFTypeRef;
    fn CFDictionaryGetValue(dictionary: CFTypeRef, key: CFTypeRef) -> CFTypeRef;
    fn CFArrayGetCount(array: CFTypeRef) -> isize;
    fn CFArrayGetValueAtIndex(array: CFTypeRef, index: isize) -> CFTypeRef;
    fn CFBooleanGetValue(boolean: CFTypeRef) -> u8;
}

#[link(name = "CoreVideo", kind = "framework")]
unsafe extern "C" {
    static kCVPixelBufferPixelFormatTypeKey: CFStringRef;
    static kCVPixelBufferWidthKey: CFStringRef;
    static kCVPixelBufferHeightKey: CFStringRef;
    static kCVPixelBufferIOSurfacePropertiesKey: CFStringRef;
    fn CVPixelBufferPoolCreatePixelBuffer(allocator: CFTypeRef, pool: CFTypeRef, buffer: *mut CFTypeRef) -> i32;
    fn CVPixelBufferLockBaseAddress(buffer: CFTypeRef, flags: u64) -> i32;
    fn CVPixelBufferUnlockBaseAddress(buffer: CFTypeRef, flags: u64) -> i32;
    fn CVPixelBufferGetBaseAddress(buffer: CFTypeRef) -> *mut u8;
    fn CVPixelBufferGetBytesPerRow(buffer: CFTypeRef) -> usize;
    static kCVImageBufferYCbCrMatrixKey: CFStringRef;
    static kCVImageBufferYCbCrMatrix_ITU_R_601_4: CFStringRef;
    static kCVImageBufferColorPrimariesKey: CFStringRef;
    static kCVImageBufferColorPrimaries_SMPTE_C: CFStringRef;
    static kCVImageBufferTransferFunctionKey: CFStringRef;
    static kCVImageBufferTransferFunction_ITU_R_709_2: CFStringRef;
    fn CVBufferSetAttachment(buffer: CFTypeRef, key: CFStringRef, value: CFTypeRef, mode: u32);
}

#[link(name = "CoreMedia", kind = "framework")]
unsafe extern "C" {
    static kCMSampleAttachmentKey_NotSync: CFStringRef;
    fn CMSampleBufferGetFormatDescription(sample: CFTypeRef) -> CFTypeRef;
    fn CMSampleBufferGetDataBuffer(sample: CFTypeRef) -> CFTypeRef;
    fn CMSampleBufferGetSampleAttachmentsArray(sample: CFTypeRef, create: u8) -> CFTypeRef;
    fn CMSampleBufferGetPresentationTimeStamp(sample: CFTypeRef) -> CMTime;
    fn CMVideoFormatDescriptionGetH264ParameterSetAtIndex(
        description: CFTypeRef,
        index: usize,
        pointer: *mut *const u8,
        size: *mut usize,
        count: *mut usize,
        header_length: *mut i32,
    ) -> OSStatus;
    fn CMBlockBufferGetDataLength(buffer: CFTypeRef) -> usize;
    fn CMBlockBufferCopyDataBytes(buffer: CFTypeRef, offset: usize, length: usize, destination: *mut u8) -> OSStatus;
}

type OutputCallback = unsafe extern "C" fn(*mut c_void, *mut c_void, OSStatus, u32, CFTypeRef);

#[link(name = "VideoToolbox", kind = "framework")]
unsafe extern "C" {
    static kVTCompressionPropertyKey_RealTime: CFStringRef;
    static kVTCompressionPropertyKey_Quality: CFStringRef;
    static kVTCompressionPropertyKey_AverageBitRate: CFStringRef;
    static kVTCompressionPropertyKey_ExpectedFrameRate: CFStringRef;
    static kVTCompressionPropertyKey_AllowFrameReordering: CFStringRef;
    static kVTCompressionPropertyKey_ProfileLevel: CFStringRef;
    static kVTCompressionPropertyKey_MaxKeyFrameInterval: CFStringRef;
    static kVTProfileLevel_H264_High_AutoLevel: CFStringRef;
    static kVTEncodeFrameOptionKey_ForceKeyFrame: CFStringRef;
    static kVTVideoEncoderSpecification_RequireHardwareAcceleratedVideoEncoder: CFStringRef;
    fn VTCompressionSessionCreate(
        allocator: CFTypeRef,
        width: i32,
        height: i32,
        codec: u32,
        specification: CFTypeRef,
        source_attributes: CFTypeRef,
        compressed_allocator: CFTypeRef,
        callback: OutputCallback,
        refcon: *mut c_void,
        session: *mut CFTypeRef,
    ) -> OSStatus;
    fn VTCompressionSessionGetPixelBufferPool(session: CFTypeRef) -> CFTypeRef;
    fn VTCompressionSessionPrepareToEncodeFrames(session: CFTypeRef) -> OSStatus;
    fn VTCompressionSessionEncodeFrame(
        session: CFTypeRef,
        image: CFTypeRef,
        time: CMTime,
        duration: CMTime,
        properties: CFTypeRef,
        source_refcon: *mut c_void,
        info_flags: *mut u32,
    ) -> OSStatus;
    fn VTCompressionSessionCompleteFrames(session: CFTypeRef, until: CMTime) -> OSStatus;
    fn VTCompressionSessionInvalidate(session: CFTypeRef);
    fn VTSessionSetProperty(session: CFTypeRef, key: CFStringRef, value: CFTypeRef) -> OSStatus;
}

/// A Core Foundation object this module owns; released with it.
struct Owned(CFTypeRef);

// SAFETY: Core Foundation objects are thread-safe to retain and release, and
// the ones held here are used by one thread at a time.
unsafe impl Send for Owned {}

impl Drop for Owned {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: the object was created or retained by this module and
            // is released once.
            unsafe { CFRelease(self.0) };
        }
    }
}

fn number_i32(value: i32) -> Owned {
    // SAFETY: the number is copied from the pointer.
    Owned(unsafe { CFNumberCreate(ptr::null(), K_CF_NUMBER_SINT32, ptr::addr_of!(value).cast()) })
}

fn number_f32(value: f32) -> Owned {
    // SAFETY: the number is copied from the pointer.
    Owned(unsafe { CFNumberCreate(ptr::null(), K_CF_NUMBER_FLOAT32, ptr::addr_of!(value).cast()) })
}

fn dictionary(entries: &[(CFTypeRef, CFTypeRef)]) -> Owned {
    let keys: Vec<CFTypeRef> = entries.iter().map(|entry| entry.0).collect();
    let values: Vec<CFTypeRef> = entries.iter().map(|entry| entry.1).collect();
    // SAFETY: the keys and values are Core Foundation objects the dictionary
    // retains.
    Owned(unsafe {
        CFDictionaryCreate(
            ptr::null(),
            keys.as_ptr(),
            values.as_ptr(),
            entries.len() as isize,
            &raw const kCFTypeDictionaryKeyCallBacks,
            &raw const kCFTypeDictionaryValueCallBacks,
        )
    })
}

fn check(status: OSStatus, what: &str) -> Result<()> {
    if status == NO_ERROR {
        Ok(())
    } else {
        Err(Error(format!("VideoToolbox: {what} failed ({status})")))
    }
}

/// What the session's callback fills and the encoder's calls empty.
#[derive(Default)]
struct Output {
    pictures: VecDeque<Picture>,
    /// The times of the pictures the encoder dropped, which no sample
    /// answers.
    dropped: Vec<u64>,
    failure: Option<String>,
}

pub struct VideoToolbox {
    session: Owned,
    output: Arc<Mutex<Output>>,
    /// The callback's context: a leaked clone of `output`, which is freed
    /// after the session is invalidated.
    refcon: *const Mutex<Output>,
    settings: Settings,
}

// SAFETY: the session is used by one thread at a time, and `refcon` only
// keeps `output` alive for the callback.
unsafe impl Send for VideoToolbox {}

impl VideoToolbox {
    pub fn start(settings: Settings) -> Result<Self> {
        let output = Arc::new(Mutex::new(Output::default()));
        let refcon = Arc::into_raw(output.clone());
        let width = number_i32(settings.width as i32);
        let height = number_i32(settings.height as i32);
        let format = number_i32(PIXEL_BGRA);
        let surface = dictionary(&[]);
        // SAFETY: statics of VideoToolbox and CoreVideo, valid for the process.
        let attributes = unsafe {
            dictionary(&[
                (kCVPixelBufferPixelFormatTypeKey, format.0),
                (kCVPixelBufferWidthKey, width.0),
                (kCVPixelBufferHeightKey, height.0),
                (kCVPixelBufferIOSurfacePropertiesKey, surface.0),
            ])
        };
        // SAFETY: as above.
        let specification = unsafe { dictionary(&[(kVTVideoEncoderSpecification_RequireHardwareAcceleratedVideoEncoder, kCFBooleanTrue)]) };
        let mut session: CFTypeRef = ptr::null();
        // SAFETY: the dictionaries are valid for the call, and `refcon` is a
        // pointer the callback only reads while the session lives.
        let status = unsafe {
            VTCompressionSessionCreate(
                ptr::null(),
                settings.width as i32,
                settings.height as i32,
                CODEC_H264,
                specification.0,
                attributes.0,
                ptr::null(),
                callback,
                refcon.cast_mut().cast(),
                &mut session,
            )
        };
        let this = Self {
            session: Owned(session),
            output,
            refcon,
            settings,
        };
        if status != NO_ERROR || session.is_null() {
            // `this` frees the context, and releases no session.
            return Err(Error(format!("VideoToolbox: no hardware encoder started ({status})")));
        }
        this.configure()?;
        // SAFETY: the session is valid.
        check(unsafe { VTCompressionSessionPrepareToEncodeFrames(this.session.0) }, "preparing the encoder")?;
        Ok(this)
    }

    fn property(&self, key: CFStringRef, value: CFTypeRef) -> Result<()> {
        // SAFETY: the session is valid and the key is one of its own.
        check(unsafe { VTSessionSetProperty(self.session.0, key, value) }, "a setting")
    }

    fn configure(&self) -> Result<()> {
        // SAFETY: statics of VideoToolbox, valid for the process.
        unsafe {
            // No picture waits for a later one: each is coded as it comes.
            self.property(kVTCompressionPropertyKey_AllowFrameReordering, kCFBooleanFalse)?;
            self.property(kVTCompressionPropertyKey_ProfileLevel, kVTProfileLevel_H264_High_AutoLevel)?;
            // Key frames come when the caller asks for one.
            self.property(kVTCompressionPropertyKey_MaxKeyFrameInterval, number_i32(i32::MAX).0)?;
            match self.settings.rate {
                Rate::Quality => {
                    self.property(kVTCompressionPropertyKey_RealTime, kCFBooleanTrue)?;
                    self.property(kVTCompressionPropertyKey_Quality, number_f32(QUALITY).0)?;
                    self.property(kVTCompressionPropertyKey_ExpectedFrameRate, number_i32(60).0)?;
                }
                Rate::Bitrate { bits_per_second, fps } => {
                    self.property(kVTCompressionPropertyKey_RealTime, kCFBooleanTrue)?;
                    self.property(kVTCompressionPropertyKey_AverageBitRate, number_i32(bits_per_second.min(i32::MAX as u32) as i32).0)?;
                    self.property(kVTCompressionPropertyKey_ExpectedFrameRate, number_i32(fps.max(1) as i32).0)?;
                }
            }
        }
        Ok(())
    }

    fn drain(&self) -> Result<Vec<Picture>> {
        let mut output = self.output.lock().expect("the encoder's output is intact");
        if let Some(failure) = output.failure.take() {
            return Err(Error(failure));
        }
        Ok(output.pictures.drain(..).collect())
    }
}

/// The quality every picture of [`Rate::Quality`] is coded at, 0 to 1. A
/// simulator scrolled without pause at 1206 × 2622 for 20 seconds made 13 MB
/// at 0.5, 24 MB at 0.65, 60 MB at 0.8 and 186 MB at 0.9, where a recording
/// ends at 200 MB; a white pixel drawn on a gradient decodes to 80% of white
/// at 0.65 and nearly all of it at 0.8. 0.8 shows a one-pixel change
/// plainly and still lasts a recording's two minutes at a pace that is not
/// all scrolling.
const QUALITY: f32 = 0.8;

impl Drop for VideoToolbox {
    fn drop(&mut self) {
        // SAFETY: the session, when there is one, is valid until invalidated
        // here, once; after that no callback runs, and the context is
        // released. A session that failed to be created has no callback to
        // wait for.
        unsafe {
            if !self.session.0.is_null() {
                VTCompressionSessionCompleteFrames(self.session.0, CMTime::INVALID);
                VTCompressionSessionInvalidate(self.session.0);
            }
            drop(Arc::from_raw(self.refcon));
        }
    }
}

impl crate::Encoder for VideoToolbox {
    fn name(&self) -> &'static str {
        "VideoToolbox"
    }

    fn encode(&mut self, frame: Raw<'_>, time_us: u64, key: bool) -> Result<Vec<Picture>> {
        let (width, height) = (frame.width as usize, frame.height as usize);
        if (frame.width, frame.height) != (self.settings.width, self.settings.height)
            || frame.pixels.len() != width * height * frame.format.bytes()
        {
            return Err(Error("a picture is not the size the encoder was started at".into()));
        }
        let mut buffer: CFTypeRef = ptr::null();
        // SAFETY: the pool belongs to the session, which is valid.
        let status = unsafe {
            CVPixelBufferPoolCreatePixelBuffer(ptr::null(), VTCompressionSessionGetPixelBufferPool(self.session.0), &mut buffer)
        };
        if status == POOL_EXHAUSTED {
            // The hardware holds every buffer: the picture is dropped, and
            // counted.
            self.output.lock().expect("the encoder's output is intact").dropped.push(time_us);
            return self.drain();
        }
        check(status, "a pixel buffer")?;
        let buffer = Owned(buffer);
        // SAFETY: the buffer is valid; the lock gives its memory, `rows` bytes
        // to the row, for `height` rows, written by `copy`.
        unsafe {
            check(CVPixelBufferLockBaseAddress(buffer.0, LOCK_WRITE), "a pixel buffer's lock")?;
            let rows = CVPixelBufferGetBytesPerRow(buffer.0);
            let base = CVPixelBufferGetBaseAddress(buffer.0);
            copy(frame, base, rows);
            CVPixelBufferUnlockBaseAddress(buffer.0, LOCK_WRITE);
        }
        // The video says BT.601, the matrix the decoder of an export reads it
        // with, and which the Host's other encoders convert with.
        // SAFETY: the buffer is valid, and the keys and values are statics of
        // CoreVideo; the attachments propagate to the encoder.
        unsafe {
            CVBufferSetAttachment(buffer.0, kCVImageBufferYCbCrMatrixKey, kCVImageBufferYCbCrMatrix_ITU_R_601_4, 1);
            CVBufferSetAttachment(buffer.0, kCVImageBufferColorPrimariesKey, kCVImageBufferColorPrimaries_SMPTE_C, 1);
            CVBufferSetAttachment(buffer.0, kCVImageBufferTransferFunctionKey, kCVImageBufferTransferFunction_ITU_R_709_2, 1);
        }
        let force = key.then(|| {
            // SAFETY: statics of VideoToolbox.
            unsafe { dictionary(&[(kVTEncodeFrameOptionKey_ForceKeyFrame, kCFBooleanTrue)]) }
        });
        // SAFETY: the session and the buffer are valid; the callback is
        // ready for the picture.
        let status = unsafe {
            VTCompressionSessionEncodeFrame(
                self.session.0,
                buffer.0,
                CMTime::microseconds(time_us),
                CMTime::INVALID,
                force.as_ref().map_or(ptr::null(), |force| force.0),
                // The picture's time rides along, so that a dropped picture
                // says which it was.
                time_us as usize as *mut c_void,
                ptr::null_mut(),
            )
        };
        check(status, "a picture")?;
        self.drain()
    }

    fn set_rate(&mut self, bits_per_second: u32, fps: u32) -> Result<()> {
        self.settings.rate = Rate::Bitrate { bits_per_second, fps };
        self.configure()
    }

    fn finish(&mut self) -> Result<Vec<Picture>> {
        // SAFETY: the session is valid.
        check(unsafe { VTCompressionSessionCompleteFrames(self.session.0, CMTime::INVALID) }, "finishing")?;
        self.drain()
    }

    fn take_dropped(&mut self) -> Vec<u64> {
        std::mem::take(&mut self.output.lock().expect("the encoder's output is intact").dropped)
    }
}

/// Writes `frame` as BGRA into `base`, `rows` bytes to the row.
///
/// # Safety
///
/// `base` must hold `frame.height` rows of `rows` bytes, each at least
/// `frame.width * 4` long.
unsafe fn copy(frame: Raw<'_>, base: *mut u8, rows: usize) {
    let (width, height) = (frame.width as usize, frame.height as usize);
    let bytes = frame.format.bytes();
    for y in 0..height {
        let source = &frame.pixels[y * width * bytes..][..width * bytes];
        // SAFETY: the caller's contract.
        let target = unsafe { std::slice::from_raw_parts_mut(base.add(y * rows), width * 4) };
        match frame.format {
            PixelFormat::Bgra => target.copy_from_slice(source),
            PixelFormat::Rgba => {
                for (to, from) in target.chunks_exact_mut(4).zip(source.chunks_exact(4)) {
                    to.copy_from_slice(&[from[2], from[1], from[0], from[3]]);
                }
            }
            PixelFormat::Rgb => {
                for (to, from) in target.chunks_exact_mut(4).zip(source.chunks_exact(3)) {
                    to.copy_from_slice(&[from[2], from[1], from[0], 255]);
                }
            }
        }
    }
}

/// The session's callback: a coded picture as Annex B, queued.
unsafe extern "C" fn callback(refcon: *mut c_void, source: *mut c_void, status: OSStatus, _flags: u32, sample: CFTypeRef) {
    // SAFETY: `refcon` is the pointer `start` leaked, which lives until the
    // session is invalidated.
    let output = unsafe { &*refcon.cast::<Mutex<Output>>() };
    let mut output = output.lock().expect("the encoder's output is intact");
    if status != NO_ERROR {
        output.failure.get_or_insert(format!("VideoToolbox: a picture did not encode ({status})"));
        return;
    }
    if sample.is_null() {
        // A picture the encoder dropped, for its bit rate or because it fell
        // behind; the source's refcon is the time it was given with.
        output.dropped.push(source as usize as u64);
        return;
    }
    // SAFETY: `sample` is a sample buffer valid for the callback.
    if let Some(picture) = unsafe { annex_b(sample) } {
        output.pictures.push_back(picture);
    } else {
        output.failure.get_or_insert("VideoToolbox gave a picture that is not H.264".into());
    }
}

/// The sample's H.264 as Annex B, with the parameter sets in front of a key
/// frame: VideoToolbox gives length-prefixed units and keeps its parameter
/// sets in the format description.
///
/// # Safety
///
/// `sample` must be a valid `CMSampleBuffer`.
unsafe fn annex_b(sample: CFTypeRef) -> Option<Picture> {
    // SAFETY: the caller's contract covers every call here.
    unsafe {
        let attachments = CMSampleBufferGetSampleAttachmentsArray(sample, 0);
        let key = if attachments.is_null() || CFArrayGetCount(attachments) == 0 {
            true
        } else {
            let first = CFArrayGetValueAtIndex(attachments, 0);
            let not_sync = CFDictionaryGetValue(first, kCMSampleAttachmentKey_NotSync);
            not_sync.is_null() || CFBooleanGetValue(not_sync) == 0
        };
        let mut data = Vec::new();
        if key {
            let description = CMSampleBufferGetFormatDescription(sample);
            let mut count = 0usize;
            CMVideoFormatDescriptionGetH264ParameterSetAtIndex(description, 0, ptr::null_mut(), ptr::null_mut(), &mut count, ptr::null_mut());
            for index in 0..count {
                let (mut set, mut size) = (ptr::null(), 0usize);
                if CMVideoFormatDescriptionGetH264ParameterSetAtIndex(description, index, &mut set, &mut size, ptr::null_mut(), ptr::null_mut())
                    == NO_ERROR
                    && !set.is_null()
                {
                    data.extend_from_slice(&[0, 0, 0, 1]);
                    data.extend_from_slice(std::slice::from_raw_parts(set, size));
                }
            }
        }
        // The units, copied whole: a sample's data may lie in several blocks.
        let block = CMSampleBufferGetDataBuffer(sample);
        if block.is_null() {
            return None;
        }
        let mut units = vec![0u8; CMBlockBufferGetDataLength(block)];
        if CMBlockBufferCopyDataBytes(block, 0, units.len(), units.as_mut_ptr()) != NO_ERROR {
            return None;
        }
        let mut at = 0;
        while at + 4 <= units.len() {
            let size = u32::from_be_bytes([units[at], units[at + 1], units[at + 2], units[at + 3]]) as usize;
            at += 4;
            if size == 0 || at + size > units.len() {
                break;
            }
            data.extend_from_slice(&[0, 0, 0, 1]);
            data.extend_from_slice(&units[at..at + size]);
            at += size;
        }
        let time = CMSampleBufferGetPresentationTimeStamp(sample);
        let time_us = if time.timescale > 0 {
            (i128::from(time.value) * 1_000_000 / i128::from(time.timescale)).max(0) as u64
        } else {
            0
        };
        Some(Picture { data, key, time_us })
    }
}
