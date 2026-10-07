//! The ClientHello the engine's upstream requests send, read off a loopback
//! socket: the fingerprint a site's edge judges before any page loads
//! (`preview.md` § Upstream requests).

use demi_command_package_browser_protocol::preview::PreviewClient;
use tokio::io::AsyncReadExt;
use tokio::net::TcpListener;

use crate::support::{CHROME_154, Relay, client, engine, opening, top};

const CHROME_149: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/149.0.0.0 Safari/537.36";
const TRUST_ANCHORS: u16 = 0xca34;
const SIGNATURE_ALGORITHMS: u16 = 0x000d;
/// Chrome 154's trust anchor IDs, as tls.peet.ws recorded them on
/// 2026-10-07: the ones the engine must send.
const CHROME_TRUST_ANCHORS: &[u8] = &[
    0x05, 0x82, 0xdf, 0x13, 0x02, 0x01, 0x05, 0x82, 0xdf, 0x13, 0x02, 0x06, 0x05, 0x82, 0xdf, 0x13, 0x02, 0x0d, 0x05, 0x82,
    0xdf, 0x13, 0x02, 0x0e, 0x05, 0x82, 0xdf, 0x13, 0x02, 0x0f, 0x05, 0x82, 0xdf, 0x13, 0x02, 0x12, 0x05, 0x82, 0xdf, 0x13,
    0x02, 0x13, 0x05, 0x82, 0xdf, 0x13, 0x02, 0x14, 0x08, 0x83, 0x9a, 0x64, 0x8c, 0x9b, 0x2d, 0x01, 0x07, 0x08, 0x83, 0x9a,
    0x64, 0x8c, 0x9b, 0x2d, 0x01, 0x08, 0x08, 0x83, 0x9a, 0x64, 0x8c, 0x9b, 0x2d, 0x01, 0x09, 0x08, 0x83, 0x9a, 0x64, 0x8c,
    0x9b, 0x2d, 0x01, 0x0a, 0x08, 0x83, 0x9a, 0x64, 0x8c, 0x9b, 0x2d, 0x01, 0x0b, 0x08, 0x83, 0x9a, 0x64, 0x8c, 0x9b, 0x2d,
    0x01, 0x0c, 0x08, 0x83, 0x9a, 0x64, 0x8c, 0x9b, 0x2d, 0x01, 0x0d, 0x08, 0x83, 0x9a, 0x64, 0x8c, 0x9b, 0x2d, 0x01, 0x12,
    0x08, 0x83, 0x9a, 0x64, 0x8c, 0x9b, 0x2d, 0x01, 0x13, 0x04, 0xd6, 0x79, 0x09, 0x01, 0x04, 0xd6, 0x79, 0x09, 0x04, 0x04,
    0xd6, 0x79, 0x09, 0x05, 0x04, 0xd6, 0x79, 0x09, 0x06, 0x04, 0xd6, 0x79, 0x09, 0x07, 0x04, 0xd6, 0x79, 0x09, 0x08, 0x04,
    0xd6, 0x79, 0x09, 0x0a, 0x04, 0xd6, 0x79, 0x09, 0x0b, 0x04, 0xd6, 0x79, 0x09, 0x0c, 0x04, 0xd6, 0x79, 0x09, 0x0d, 0x04,
    0xd6, 0x79, 0x09, 0x0f,
];
/// ML-DSA-44, -65 and -87, which Chrome 154 lists after a GREASE value.
const CHROME_ADVERTISED_SIGALGS: [u16; 3] = [0x0904, 0x0905, 0x0906];

/// The extensions of the first ClientHello the user's opening of a loopback
/// address sends, from a browser with `user_agent`, by type, in order.
async fn client_hello(user_agent: &str) -> Vec<(u16, Vec<u8>)> {
    let directory = tempfile::tempdir().unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("https://{}", listener.local_addr().unwrap());
    let mut relay = Relay::open(engine(&directory));
    relay.client = PreviewClient {
        user_agent: user_agent.into(),
        ..client()
    };
    let opened = tokio::spawn(async move { relay.fetch(top(&origin), opening(&format!("{origin}/"))).await });
    let (mut socket, _) = listener.accept().await.unwrap();
    let mut header = [0; 5];
    socket.read_exact(&mut header).await.unwrap();
    assert_eq!(header[0], 22, "a TLS handshake record");
    let mut record = vec![0; usize::from(u16::from_be_bytes([header[3], header[4]]))];
    socket.read_exact(&mut record).await.unwrap();
    // The listener closes after the hello: the request fails, which is not
    // what the test is about.
    drop(socket);
    drop(listener);
    assert!(opened.await.unwrap().is_err());
    extensions(&record)
}

/// The extensions of a ClientHello handshake message.
fn extensions(message: &[u8]) -> Vec<(u16, Vec<u8>)> {
    assert_eq!(message[0], 1, "a ClientHello");
    // Type, length, version and random.
    let mut at = 4 + 2 + 32;
    let session = usize::from(message[at]);
    at += 1 + session;
    let ciphers = usize::from(u16::from_be_bytes([message[at], message[at + 1]]));
    at += 2 + ciphers;
    let compression = usize::from(message[at]);
    at += 1 + compression;
    let end = at + 2 + usize::from(u16::from_be_bytes([message[at], message[at + 1]]));
    at += 2;
    let mut extensions = Vec::new();
    while at < end {
        let kind = u16::from_be_bytes([message[at], message[at + 1]]);
        let length = usize::from(u16::from_be_bytes([message[at + 2], message[at + 3]]));
        extensions.push((kind, message[at + 4..at + 4 + length].to_vec()));
        at += 4 + length;
    }
    extensions
}

fn extension(extensions: &[(u16, Vec<u8>)], kind: u16) -> Option<&[u8]> {
    extensions.iter().find(|(found, _)| *found == kind).map(|(_, data)| data.as_slice())
}

/// The signature algorithms the hello lists, in order.
fn signature_algorithms(extensions: &[(u16, Vec<u8>)]) -> Vec<u16> {
    let data = extension(extensions, SIGNATURE_ALGORITHMS).expect("signature_algorithms");
    data[2..].chunks(2).map(|pair| u16::from_be_bytes([pair[0], pair[1]])).collect()
}

/// A GREASE code point (RFC 8701): 0x?A?A with both bytes equal.
fn is_grease(value: u16) -> bool {
    let [high, low] = value.to_be_bytes();
    high == low && high & 0x0f == 0x0a
}

#[tokio::test]
async fn a_chrome_154_user_agent_sends_chromes_trust_anchors_and_signature_algorithms() {
    let hello = client_hello(CHROME_154).await;
    let anchors = extension(&hello, TRUST_ANCHORS).expect("the trust anchors extension");
    // The extension carries the IDs behind a two-byte length.
    assert_eq!(&anchors[2..], CHROME_TRUST_ANCHORS);
    let algorithms = signature_algorithms(&hello);
    assert!(is_grease(algorithms[0]), "{algorithms:04x?}");
    assert_eq!(algorithms[1..4], CHROME_ADVERTISED_SIGALGS, "{algorithms:04x?}");
    // ECDSA P-256 SHA-256, the first of Chrome's verification preferences.
    assert_eq!(algorithms[4], 0x0403, "{algorithms:04x?}");
}

#[tokio::test]
async fn an_older_chrome_user_agent_sends_its_profiles_hello_unchanged() {
    let hello = client_hello(CHROME_149).await;
    assert!(extension(&hello, TRUST_ANCHORS).is_none());
    let algorithms = signature_algorithms(&hello);
    assert_eq!(algorithms[0], 0x0403, "{algorithms:04x?}");
    assert!(!algorithms.iter().copied().any(is_grease), "{algorithms:04x?}");
}
