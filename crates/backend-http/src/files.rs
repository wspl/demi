//! A conversation's files (`web-api.md` § Device files and remote
//! references, § File text and working tree changes): listings, directories,
//! deletes and file text, a file's bytes in both directions, and the working
//! tree's changes. Each reaches the conversation's Host through its host
//! access (`sessions-and-targets.md` § Host operations), in the user's
//! shard, which also checks that the conversation is the caller's; the bytes
//! of a download or an upload move here while the shard holds their
//! admission.

use axum::Json;
use axum::body::Body;
use axum::extract::{ConnectInfo, Path, State};
use axum::http::header::{CONTENT_LENGTH, ETAG, IF_NONE_MATCH, LAST_MODIFIED, RANGE};
use axum::http::{HeaderMap, HeaderName, HeaderValue, Method, StatusCode};
use axum::response::{IntoResponse, Response};
use demi_backend_remote_host::{ConditionalRead, byte_stream};
use demi_backend_runners::files::{
    TEXT_RANGE, TextError, browse_directory, read_text_file, text_of, text_of_stream,
};
use demi_host_interface::{FileStat, HostFs, MkdirOptions, RmOptions};
use demi_runner_protocol::files::protected_path;
use demi_shared_types::preview_media_type;
use demi_web_api_protocol::error::ErrorCode;
use demi_web_api_protocol::files::{
    ChangeSides, CommittedFileQuery, CreateDirectory, CreatedDirectory, Directory, DirectoryQuery,
    FileQuery, FileText, RawFileQuery, RemoveQuery, TreeFileQuery, UploadQuery, WorkingTreeChanges,
};
use demi_web_api_protocol::ids::{ConversationId, DeviceId, UserId};
use typed_path::Utf8TypedPath;

use super::AppState;
use super::body::JsonBody;
use super::content::content_headers;
use super::error::ApiError;
use super::gate::AuthUser;
use super::listener::Peer;
use super::query::QueryParams;
use super::transfer::{TRANSFER_IDLE, UploadEnd, copy_upload, paced_body};
use demi_backend_host_access::access::{ConversationHost, HostAccessError, Refusal};
use demi_backend_host_access::transfer::{Download, DownloadRequest, RangeAnswer};

pub(super) async fn list(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Path(id): Path<String>,
    QueryParams(query): QueryParams<DirectoryQuery>,
) -> Result<Json<Directory>, ApiError> {
    list_on(&state, &user.id, &id, None, query).await
}

/// A directory on one of the conversation's attached Hosts.
pub(super) async fn list_on_host(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Path((id, device)): Path<(String, String)>,
    QueryParams(query): QueryParams<DirectoryQuery>,
) -> Result<Json<Directory>, ApiError> {
    list_on(&state, &user.id, &id, Some(&device), query).await
}

/// A directory's entries, the Host's starting directory when the query
/// names none.
async fn list_on(
    state: &AppState,
    user: &UserId,
    id: &str,
    device: Option<&str>,
    query: DirectoryQuery,
) -> Result<Json<Directory>, ApiError> {
    let requested = query.path.map(|path| path.as_str().to_owned());
    let directory = on_host(state, user, id, device, async move |host| {
        let path = requested.unwrap_or_else(|| host.root.clone());
        let entries = browse_directory(&host.host, &path)
            .await
            .map_err(ApiError::host_operation)?;
        Ok(Directory {
            path,
            home: host.home.clone(),
            entries,
        })
    })
    .await?;
    Ok(Json(directory))
}

pub(super) async fn make_directory(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Path(id): Path<String>,
    JsonBody(body): JsonBody<CreateDirectory>,
) -> Result<(StatusCode, Json<CreatedDirectory>), ApiError> {
    make_directory_on(&state, &user.id, &id, None, body).await
}

pub(super) async fn make_directory_on_host(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Path((id, device)): Path<(String, String)>,
    JsonBody(body): JsonBody<CreateDirectory>,
) -> Result<(StatusCode, Json<CreatedDirectory>), ApiError> {
    make_directory_on(&state, &user.id, &id, Some(&device), body).await
}

/// Makes a directory, with its parents.
async fn make_directory_on(
    state: &AppState,
    user: &UserId,
    id: &str,
    device: Option<&str>,
    CreateDirectory { path }: CreateDirectory,
) -> Result<(StatusCode, Json<CreatedDirectory>), ApiError> {
    let made = on_host(state, user, id, device, async move |host| {
        HostFs::mkdir(&host.host, &path, MkdirOptions { recursive: true })
            .await
            .map_err(ApiError::host_operation)?;
        Ok(CreatedDirectory { path })
    })
    .await?;
    Ok((StatusCode::CREATED, Json(made)))
}

/// Deletes a file, or a directory with what is in it. Nothing at the path
/// is nothing to delete.
pub(super) async fn remove(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Path(id): Path<String>,
    QueryParams(RemoveQuery { path }): QueryParams<RemoveQuery>,
) -> Result<StatusCode, ApiError> {
    on_host(&state, &user.id, &id, None, async move |host| {
        let kept = [host.home.as_deref(), Some(host.root.as_str())];
        if protected_path(path.as_str(), kept.into_iter().flatten()) {
            return Err(ApiError::new(
                StatusCode::CONFLICT,
                ErrorCode::ProtectedPath,
                "The root, the home directory and the execution directory stay, with every directory holding them",
            ));
        }
        let options = RmOptions {
            recursive: true,
            force: true,
        };
        HostFs::rm(&host.host, path.as_str(), options)
            .await
            .map_err(ApiError::host_operation)?;
        Ok(StatusCode::NO_CONTENT)
    })
    .await
}

/// What a text read answers: the text with its version, or that the file
/// still has the version the request named.
enum TextRead {
    Text { version: String, file: FileText },
    NotModified { version: String },
}

/// A file's text: UTF-8 without a NUL byte, up to the size an edit keeps,
/// with its version as the ETag. A request whose `If-None-Match` names the
/// file's version answers 304, and its text never leaves the Host
/// (`runner.md` § File contents).
pub(super) async fn text(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    headers: HeaderMap,
    Path(id): Path<String>,
    QueryParams(FileQuery { path }): QueryParams<FileQuery>,
) -> Result<Response, ApiError> {
    let held = header(&headers, IF_NONE_MATCH).map(|held| held.trim().to_owned());
    let read = on_host(&state, &user.id, &id, None, async move |host| {
        let path = path.as_str().to_owned();
        let read = match held.as_deref() {
            Some(held) => host.host.read_pipe_unless(&path, TEXT_RANGE, held).await,
            None => host
                .host
                .read_pipe(&path, TEXT_RANGE)
                .await
                .map(ConditionalRead::Opened),
        };
        match read.map_err(TextError::Host)? {
            ConditionalRead::Unchanged { version, .. } => Ok(TextRead::NotModified { version }),
            ConditionalRead::Opened(opened) => {
                let text = text_of_stream(byte_stream(opened.body)).await?;
                Ok(TextRead::Text {
                    version: opened.version,
                    file: FileText { path, text },
                })
            }
        }
    })
    .await?;
    Ok(match read {
        TextRead::Text { version, file } => {
            ([(ETAG, header_value(&version))], Json(file)).into_response()
        }
        TextRead::NotModified { version } => {
            (StatusCode::NOT_MODIFIED, [(ETAG, header_value(&version))]).into_response()
        }
    })
}

/// A file's bytes as a transfer (§ Host operations): by range, under the
/// headers that keep them inert, and cut short rather than ended when the
/// transfer ends early.
pub(super) async fn raw(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    ConnectInfo(peer): ConnectInfo<Peer>,
    method: Method,
    headers: HeaderMap,
    Path(id): Path<String>,
    QueryParams(query): QueryParams<RawFileQuery>,
) -> Result<Response, ApiError> {
    let conversation = conversation_id(&id)?;
    let path = query.path.as_str().to_owned();
    // The bytes of a version a request names can only be that version's.
    let versioned = query.version.is_some();
    let request = DownloadRequest {
        path: path.clone(),
        version: query.version.map(|version| version.as_str().to_owned()),
        head: method == Method::HEAD,
        range: header(&headers, RANGE),
        if_none_match: header(&headers, IF_NONE_MATCH),
    };
    let download = state
        .shards
        .of(&user.id)
        .call(move |shard, cancel| async move {
            shard
                .host_shard()
                .open_download(&conversation, request, &cancel)
                .await
        })
        .await??;
    let download_flag = query.download.0;
    match download {
        Download::NotAFile => Err(ApiError::new(
            StatusCode::NOT_FOUND,
            ErrorCode::NotFound,
            "Not a regular file",
        )),
        Download::Changed => Err(ApiError::new(
            StatusCode::PRECONDITION_FAILED,
            ErrorCode::FileChanged,
            "The file is no longer the version asked for",
        )),
        Download::NotModified { version } => {
            let mut headers = raw_file_headers(versioned);
            headers.insert(ETAG, header_value(&version));
            Ok((StatusCode::NOT_MODIFIED, headers).into_response())
        }
        Download::Head {
            stat,
            version,
            part,
        } => {
            let file = FileAnswer {
                path: &path,
                download: download_flag,
                stat: &stat,
                version: &version,
                versioned,
            };
            Ok((part.status(), file.headers(&part)).into_response())
        }
        Download::Stream {
            stat,
            version,
            part,
            body,
            lease,
        } => {
            let file = FileAnswer {
                path: &path,
                download: download_flag,
                stat: &stat,
                version: &version,
                versioned,
            };
            let mut headers = file.headers(&part);
            // Streamed, the answer has no length, which a HEAD reports.
            headers.remove(CONTENT_LENGTH);
            // From here a connection nothing moves on is closed; a Cloud that
            // woke for the admission did not count.
            let watch = peer.control.watch(TRANSFER_IDLE, lease.ending());
            Ok((part.status(), headers, paced_body(body, lease, watch)).into_response())
        }
    }
}

/// An upload (§ Host operations): the request's body streams into the file
/// as the Host writes it, and the file is in place whole or not at all.
pub(super) async fn upload(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    ConnectInfo(peer): ConnectInfo<Peer>,
    Path(id): Path<String>,
    QueryParams(UploadQuery { path, replace }): QueryParams<UploadQuery>,
    body: Body,
) -> Result<StatusCode, ApiError> {
    let conversation = conversation_id(&id)?;
    let path = path.as_str().to_owned();
    let open = state
        .shards
        .of(&user.id)
        .call(move |shard, cancel| async move {
            shard
                .host_shard()
                .open_upload(&conversation, path, replace.0, &cancel)
                .await
        })
        .await??;
    match copy_upload(body, open).await {
        UploadEnd::Written => Ok(StatusCode::NO_CONTENT),
        UploadEnd::HostStalled => {
            // Nothing moved on the connection for the limit.
            peer.control.close();
            Err(ApiError::new(
                StatusCode::SERVICE_UNAVAILABLE,
                ErrorCode::HostOperationFailed,
                "The Host took nothing for too long",
            ))
        }
        // What the path holds refuses the write as the Host writes it.
        UploadEnd::WriteFailed(error) => Err(match error.code() {
            Some("EISDIR") => ApiError::new(
                StatusCode::CONFLICT,
                ErrorCode::IsDirectory,
                "A directory is at this path",
            ),
            Some("EEXIST") => ApiError::new(
                StatusCode::CONFLICT,
                ErrorCode::FileExists,
                "A file is already at this path",
            ),
            _ => ApiError::host_operation(error),
        }),
        UploadEnd::Refused(error) => Err(error),
    }
}

/// The working tree's uncommitted changes under the conversation's
/// directory.
pub(super) async fn changes(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Response, ApiError> {
    let changes = on_host(&state, &user.id, &id, None, async move |host| {
        let changes = host
            .host
            .git_changes(&host.root)
            .await
            .map_err(ApiError::working_tree)?;
        Ok(WorkingTreeChanges {
            root: host.root.clone(),
            changes,
        })
    })
    .await?;
    Ok(unless_held(&headers, &changes))
}

/// One changed file's two sides as text: the last commit's, empty for a
/// new file, and the working tree's, empty for a deleted one.
pub(super) async fn changed_file(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    headers: HeaderMap,
    Path(id): Path<String>,
    QueryParams(TreeFileQuery { path }): QueryParams<TreeFileQuery>,
) -> Result<Response, ApiError> {
    let sides = on_host(&state, &user.id, &id, None, async move |host| {
        // The two sides do not wait for each other (`runner.md` § Host
        // operations).
        let working = format!("{}/{}", host.root, path.as_str());
        let (committed, working) = tokio::join!(
            host.host.git_show(&host.root, path.as_str()),
            read_text_file(&host.host, &working),
        );
        let original = match committed {
            Ok(bytes) => text_of(Vec::from(bytes))?,
            Err(error) if error.code() == Some("ENOENT") => String::new(),
            Err(error) => return Err(ApiError::working_tree(error)),
        };
        let modified = match working {
            Ok(text) => text,
            Err(TextError::Host(error)) if error.code() == Some("ENOENT") => String::new(),
            Err(error) => return Err(error.into()),
        };
        Ok(ChangeSides { original, modified })
    })
    .await?;
    Ok(unless_held(&headers, &sides))
}

/// `value` as JSON with an ETag of its content, or 304 when the request's
/// `If-None-Match` names that ETag: the page then keeps what it holds and
/// receives nothing (`web-api.md` § File text and working tree changes).
fn unless_held(headers: &HeaderMap, value: &impl serde::Serialize) -> Response {
    use sha2::{Digest as _, Sha256};
    let body = serde_json::to_vec(value).expect("an answer of the API serializes");
    // As the assets' tags are made (`assets.rs`).
    let tag = format!("\"{:x}\"", Sha256::digest(&body));
    let held = header(headers, IF_NONE_MATCH);
    if held.is_some_and(|held| held.split(',').any(|one| one.trim() == tag)) {
        return (StatusCode::NOT_MODIFIED, [(ETAG, header_value(&tag))]).into_response();
    }
    (
        [
            (ETAG, header_value(&tag)),
            (axum::http::header::CONTENT_TYPE, HeaderValue::from_static("application/json")),
        ],
        body,
    )
        .into_response()
}

/// A changed file's last committed bytes, by range. Git's copy is read whole
/// on the runner and is at most 8 MiB, so it is held here and served whole.
pub(super) async fn committed(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    method: Method,
    headers: HeaderMap,
    Path(id): Path<String>,
    QueryParams(CommittedFileQuery { path, download }): QueryParams<CommittedFileQuery>,
) -> Result<Response, ApiError> {
    let tree_path = path.as_str().to_owned();
    let bytes = on_host(&state, &user.id, &id, None, async move |host| {
        host.host
            .git_show(&host.root, &tree_path)
            .await
            .map_err(ApiError::working_tree)
    })
    .await?;
    let size = u64::try_from(bytes.len()).expect("a file held in memory fits u64");
    let part = RangeAnswer::of(header(&headers, RANGE).as_deref(), size);
    let mut answer = raw_file_headers(false);
    answer.extend(content_headers(
        preview_media_type(path.as_str()),
        download.0,
        file_name(path.as_str()).as_deref(),
    ));
    answer.extend(part.headers());
    match part.part_of(&bytes) {
        Some(body) if method != Method::HEAD => Ok((part.status(), answer, body).into_response()),
        _ => Ok((part.status(), answer).into_response()),
    }
}

/// Runs `operation` on the conversation's Host through its host access: the
/// primary Host, or the bound device `device` names.
async fn on_host<T, F>(
    state: &AppState,
    user: &UserId,
    id: &str,
    device: Option<&str>,
    operation: F,
) -> Result<T, ApiError>
where
    T: Send + 'static,
    F: AsyncFnOnce(&ConversationHost) -> Result<T, ApiError> + Send + 'static,
{
    let conversation = conversation_id(id)?;
    let device = device
        .map(|device| {
            DeviceId::try_from(device)
                .map_err(|_| ApiError::from(HostAccessError::from(Refusal::NotAttached)))
        })
        .transpose()?;
    state
        .shards
        .of(user)
        .call(move |shard, cancel| async move {
            shard
                .host_shard()
                .with_host(&conversation, device.as_ref(), &cancel, operation)
                .await?
        })
        .await?
}

/// The path's conversation id; one that is not an id names no conversation.
fn conversation_id(id: &str) -> Result<ConversationId, ApiError> {
    ConversationId::try_from(id).map_err(|_| HostAccessError::Missing.into())
}

fn header(headers: &HeaderMap, name: HeaderName) -> Option<String> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned)
}

fn header_value(text: &str) -> HeaderValue {
    HeaderValue::from_str(text).expect("a version is printable ASCII")
}

/// The file's name, which a download is saved under.
fn file_name(path: &str) -> Option<String> {
    Utf8TypedPath::derive(path).file_name().map(str::to_owned)
}

/// Headers every raw file answer carries: private to the signed-in user,
/// revalidated on each use unless the request named the `versioned` bytes,
/// which can only be that version's, and streamed rather than buffered by a
/// proxy in front.
fn raw_file_headers(versioned: bool) -> HeaderMap {
    let mut headers = HeaderMap::new();
    let cache = if versioned {
        "private, max-age=31536000, immutable"
    } else {
        "private, no-cache"
    };
    headers.insert(
        axum::http::header::CACHE_CONTROL,
        HeaderValue::from_static(cache),
    );
    headers.insert(axum::http::header::VARY, HeaderValue::from_static("Cookie"));
    headers.insert(
        HeaderName::from_static("x-accel-buffering"),
        HeaderValue::from_static("no"),
    );
    headers
}

/// The file a raw answer serves a part of.
struct FileAnswer<'a> {
    path: &'a str,
    download: bool,
    stat: &'a FileStat,
    version: &'a str,
    /// The request named the version.
    versioned: bool,
}

impl FileAnswer<'_> {
    /// A part's headers: how its bytes may be shown, their version, and the
    /// part.
    fn headers(&self, part: &RangeAnswer) -> HeaderMap {
        let mut headers = raw_file_headers(self.versioned);
        headers.extend(content_headers(
            preview_media_type(self.path),
            self.download,
            file_name(self.path).as_deref(),
        ));
        headers.insert(ETAG, header_value(self.version));
        headers.insert(LAST_MODIFIED, last_modified(self.stat));
        headers.extend(part.headers());
        headers
    }
}

/// When the file was last modified, as an HTTP date.
fn last_modified(stat: &FileStat) -> HeaderValue {
    let date = stat
        .modified
        .to_jiff()
        .strftime("%a, %d %b %Y %H:%M:%S GMT")
        .to_string();
    HeaderValue::from_str(&date).expect("an HTTP date is a header value")
}

#[cfg(test)]
mod tests {
    use demi_host_interface::FileKind;
    use demi_shared_types::Timestamp;

    use super::*;

    #[test]
    fn a_file_is_dated_by_its_time_and_named_by_its_last_segment() {
        let stat = FileStat {
            kind: FileKind::File,
            mode: 0o644,
            size: 300_000,
            modified: Timestamp::from_millisecond(1_790_000_000_123).unwrap(),
        };
        assert_eq!(last_modified(&stat), "Mon, 21 Sep 2026 14:13:20 GMT");
        assert_eq!(file_name("/work/logo.svg").as_deref(), Some("logo.svg"));
        assert_eq!(
            file_name("C:\\work\\report.pdf").as_deref(),
            Some("report.pdf")
        );
    }
}
