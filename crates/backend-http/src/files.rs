//! A conversation's files: listings, directories, deletes and file text, a
//! file's bytes in both directions, and the working tree's changes.
//!
//! Each reaches the conversation's device through its device access
//! (`crates/backend-device-access/src/access.rs`), in the user's shard, which
//! also checks that the conversation is the caller's. The bytes of a download
//! or an upload move here while the shard holds their admission.

use axum::Json;
use axum::body::Body;
use axum::extract::{ConnectInfo, Path, State};
use axum::http::header::{CONTENT_LENGTH, ETAG, IF_NONE_MATCH, LAST_MODIFIED, RANGE};
use axum::http::{HeaderMap, HeaderName, HeaderValue, Method, StatusCode};
use axum::response::{IntoResponse, Response};
use demi_runner_client::{ConditionalRead, byte_stream};
use demi_backend_runners::files::{
    TEXT_RANGE, TextError, TextRefusal, browse_directory, text_of, text_of_stream,
};
use demi_device_interface::{FileStat, DeviceError, DeviceFs, MkdirOptions, RmOptions};
use demi_shared_types::ReadAt;
use demi_runner_protocol::files::protected_path;
use demi_shared_types::preview_media_type;
use demi_web_api_protocol::error::ErrorCode;
use demi_web_api_protocol::files::{
    AbsolutePath, ChangeSides, ChangesQuery, CommitName, CommitQuery, CommittedFileQuery, CreateDirectory, CreatedDirectory, Directory, DirectoryLists, DirectoryQuery, ListedDirectories, ListedDirectory, LogQuery,
    FileQuery, FileText, RawFileQuery, RemoveQuery, RepositoriesRequest, SearchRequest, TreeFileQuery,
    UploadQuery, WorkingTreeChanges,
};
use demi_web_api_protocol::ids::{ConversationId, DeviceId, UserId};
use typed_path::Utf8TypedPath;

use super::AppState;
use super::body::JsonBody;
use super::content::content_headers;
use super::error::{ApiError, READ_AT, read_at_value};
use super::gate::AuthUser;
use super::listener::Peer;
use super::query::QueryParams;
use super::transfer::{TRANSFER_IDLE, UploadEnd, copy_upload, paced_body};
use demi_backend_device_access::access::{ConversationDevice, DeviceAccessError, Refusal};
use demi_backend_device_access::transfer::{
    Download, DownloadPath, DownloadRequest, OpenSearch, RangeAnswer,
};

pub(super) async fn list(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Path(id): Path<String>,
    QueryParams(query): QueryParams<DirectoryQuery>,
) -> Result<Response, ApiError> {
    list_on(&state, &user.id, &id, None, query).await
}

/// A directory on one of the conversation's attached devices.
pub(super) async fn list_on_device(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Path((id, device)): Path<(String, String)>,
    QueryParams(query): QueryParams<DirectoryQuery>,
) -> Result<Response, ApiError> {
    list_on(&state, &user.id, &id, Some(&device), query).await
}

/// A directory's entries, the device's starting directory when the query
/// names none, read at the device's number it answers.
async fn list_on(
    state: &AppState,
    user: &UserId,
    id: &str,
    device: Option<&str>,
    query: DirectoryQuery,
) -> Result<Response, ApiError> {
    let requested = query.path.map(|path| path.as_str().to_owned());
    let (directory, read_at) = on_device(state, user, id, device, async move |admitted| {
        let path = requested.map_or_else(|| admitted.root.clone(), |path| admitted.path(&path));
        let (entries, read_at) = browse_directory(&admitted.device, &path)
            .await
            .map_err(ApiError::device_operation)?;
        let directory = Directory {
            path,
            home: admitted.home.clone(),
            entries,
        };
        Ok((directory, read_at))
    })
    .await?;
    Ok(read_answer(&read_at, Json(directory)))
}

/// `answer` with the device's number before it was read, as every answer of
/// what a subscription covers carries it.
pub(super) fn read_answer(read_at: &ReadAt, answer: impl IntoResponse) -> Response {
    let mut response = answer.into_response();
    if let Some(value) = read_at_value(read_at) {
        response.headers_mut().insert(READ_AT, value);
    }
    response
}

/// Several directories' entries, read from the device together: each answers as
/// `GET .../fs` would, in the order asked, and one that cannot be listed
/// leaves the others.
pub(super) async fn lists(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Path(id): Path<String>,
    JsonBody(DirectoryLists { paths }): JsonBody<DirectoryLists>,
) -> Result<Json<ListedDirectories>, ApiError> {
    let lists = on_device(&state, &user.id, &id, None, async move |device| {
        let paths: Vec<String> = paths.iter().map(|path| device.path(path)).collect();
        let reads = paths.iter().map(|path| browse_directory(&device.device, path));
        let answers = futures_util::future::join_all(reads).await;
        Ok(paths
            .into_iter()
            .zip(answers)
            .map(|(path, answer)| match answer {
                Ok((entries, read_at)) => ListedDirectory::Listed {
                    path,
                    home: device.home.clone(),
                    entries,
                    read_at: Some(read_at.to_string()),
                },
                Err(error) => {
                    let (status, error) = ApiError::device_operation(error).into_body();
                    ListedDirectory::Refused {
                        path,
                        status: status.as_u16(),
                        error,
                    }
                }
            })
            .collect())
    })
    .await?;
    Ok(Json(ListedDirectories { lists }))
}

pub(super) async fn make_directory(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Path(id): Path<String>,
    JsonBody(body): JsonBody<CreateDirectory>,
) -> Result<(StatusCode, Json<CreatedDirectory>), ApiError> {
    make_directory_on(&state, &user.id, &id, None, body).await
}

pub(super) async fn make_directory_on_device(
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
    let made = on_device(state, user, id, device, async move |device| {
        let path = device.path(&path);
        DeviceFs::mkdir(&device.device, &path, MkdirOptions { recursive: true })
            .await
            .map_err(ApiError::device_operation)?;
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
    on_device(&state, &user.id, &id, None, async move |device| {
        let kept = [device.home.as_deref(), Some(device.root.as_str())];
        let path = device.rooted(path.as_str()).map_err(DeviceAccessError::from)?;
        if protected_path(&path, kept.into_iter().flatten()) {
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
        DeviceFs::rm(&device.device, &path, options)
            .await
            .map_err(ApiError::device_operation)?;
        Ok(StatusCode::NO_CONTENT)
    })
    .await
}

/// What a text read answers: the text with its version, or that the file
/// still has the version the request named, each read at a number.
enum TextRead {
    Text { version: String, file: FileText },
    NotModified { version: String },
}

/// A file's text: UTF-8 without a NUL byte, up to the size an edit keeps, with
/// its version as the ETag.
///
/// A request whose `If-None-Match` names the file's version answers 304, and
/// its text never leaves the device (`crates/runner-device/src/files.rs`). A file
/// that is binary or too large is an answer like any other, with its ETag and
/// number.
pub(super) async fn text(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    headers: HeaderMap,
    Path(id): Path<String>,
    QueryParams(FileQuery { path }): QueryParams<FileQuery>,
) -> Result<Response, ApiError> {
    let held = header(&headers, IF_NONE_MATCH).map(|held| held.trim().to_owned());
    let (read, read_at) = on_device(&state, &user.id, &id, None, async move |device| {
        let path = device.path(path.as_str());
        let read = match held.as_deref() {
            Some(held) => device.device.read_pipe_unless(&path, TEXT_RANGE, held).await,
            None => device
                .device
                .read_pipe(&path, TEXT_RANGE)
                .await
                .map(ConditionalRead::Opened),
        };
        match read.map_err(ApiError::device_operation)? {
            ConditionalRead::Unchanged {
                version, read_at, ..
            } => Ok((TextRead::NotModified { version }, read_at)),
            ConditionalRead::Opened(opened) => {
                let text = match text_of_stream(byte_stream(opened.body)).await {
                    Ok(text) => text,
                    Err(TextError::Refused(refusal)) => {
                        return Err(ApiError::from(refusal)
                            .with_etag(opened.version)
                            .with_read_at(opened.read_at));
                    }
                    Err(TextError::Device(error)) => return Err(ApiError::device_operation(error)),
                };
                let read = TextRead::Text {
                    version: opened.version,
                    file: FileText { path, text },
                };
                Ok((read, opened.read_at))
            }
        }
    })
    .await?;
    Ok(match read {
        TextRead::Text { version, file } => {
            read_answer(&read_at, ([(ETAG, header_value(&version))], Json(file)))
        }
        TextRead::NotModified { version } => read_answer(
            &read_at,
            (StatusCode::NOT_MODIFIED, [(ETAG, header_value(&version))]),
        ),
    })
}

/// A file's bytes as a transfer (§ device operations): by range, under the
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
        path: DownloadPath::Named(path.clone()),
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
                .device_shard()
                .open_download(&conversation, request, &cancel)
                .await
        })
        .await??;
    let content = content_headers(
        preview_media_type(&path),
        query.download.0,
        file_name(&path).as_deref(),
    );
    download_answer(download, &peer, versioned, content)
}

/// What a read of a file's bytes answers, with the `content` headers that say
/// how they may be shown; `versioned` when the request named their version.
pub(super) fn download_answer(
    download: Download,
    peer: &Peer,
    versioned: bool,
    content: HeaderMap,
) -> Result<Response, ApiError> {
    match download {
        Download::NotAFile => Err(ApiError::new(
            StatusCode::NOT_FOUND,
            ErrorCode::NotFound,
            "Not a regular file",
        )),
        Download::NoPath => Err(ApiError::new(
            StatusCode::NOT_FOUND,
            ErrorCode::NotFound,
            "No such file",
        )),
        Download::Changed => Err(ApiError::new(
            StatusCode::PRECONDITION_FAILED,
            ErrorCode::FileChanged,
            "The file is no longer the version asked for",
        )),
        Download::NotModified { version, read_at } => {
            let mut headers = raw_file_headers(versioned);
            headers.insert(ETAG, header_value(&version));
            Ok(read_answer(&read_at, (StatusCode::NOT_MODIFIED, headers)))
        }
        Download::Head {
            stat,
            version,
            part,
            read_at,
        } => {
            let file = FileAnswer {
                content: &content,
                stat: &stat,
                version: &version,
                versioned,
            };
            Ok(read_answer(&read_at, (part.status(), file.headers(&part))))
        }
        Download::Stream {
            stat,
            version,
            part,
            body,
            lease,
            read_at,
        } => {
            let file = FileAnswer {
                content: &content,
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
            let body = paced_body(body, lease, watch);
            Ok(read_answer(&read_at, (part.status(), headers, body)))
        }
    }
}

/// A search of the names and the text of the files under a directory, the
/// conversation's working directory without one
/// (`crates/runner-device/src/search.rs`): its lines as the device finds them, one
/// JSON object per line, streamed as a download is, so the user's browser
/// that leaves stops the search.
pub(super) async fn search(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    ConnectInfo(peer): ConnectInfo<Peer>,
    Path(id): Path<String>,
    JsonBody(SearchRequest { root, query }): JsonBody<SearchRequest>,
) -> Result<Response, ApiError> {
    let conversation = conversation_id(&id)?;
    let root = root.map(|root| root.as_str().to_owned());
    let opened = state
        .shards
        .of(&user.id)
        .call(move |shard, cancel| async move {
            shard
                .device_shard()
                .open_search(&conversation, root, query, &cancel)
                .await
        })
        .await?;
    let OpenSearch { body, lease } = opened?;
    // From here a connection nothing moves on is closed, as a download's.
    let watch = peer.control.watch(TRANSFER_IDLE, lease.ending());
    let headers = [
        (axum::http::header::CONTENT_TYPE, HeaderValue::from_static("application/x-ndjson")),
        (axum::http::header::CACHE_CONTROL, HeaderValue::from_static("no-store")),
    ];
    Ok((headers, paced_body(body, lease, watch)).into_response())
}

/// An upload (§ device operations): the request's body streams into the file
/// as the device writes it, and the file is in place whole or not at all.
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
                .device_shard()
                .open_upload(&conversation, path, replace.0, &cancel)
                .await
        })
        .await??;
    match copy_upload(body, open).await {
        UploadEnd::Written => Ok(StatusCode::NO_CONTENT),
        UploadEnd::DeviceStalled => {
            // Nothing moved on the connection for the limit.
            peer.control.close();
            Err(ApiError::new(
                StatusCode::SERVICE_UNAVAILABLE,
                ErrorCode::DeviceOperationFailed,
                "The device took nothing for too long",
            ))
        }
        // What the path holds refuses the write as the device writes it.
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
            _ => ApiError::device_operation(error),
        }),
        UploadEnd::Refused(error) => Err(error),
    }
}

/// The working tree's root a request names, or the conversation's execution
/// directory.
fn tree_root(device: &ConversationDevice, directory: Option<AbsolutePath>) -> String {
    directory.map_or_else(|| device.root.clone(), |directory| directory.as_str().to_owned())
}

/// The working tree's uncommitted changes under the directory the request
/// names, or the conversation's.
pub(super) async fn changes(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Path(id): Path<String>,
    QueryParams(ChangesQuery { directory }): QueryParams<ChangesQuery>,
) -> Result<Response, ApiError> {
    let (changes, read_at) = on_device(&state, &user.id, &id, None, async move |device| {
        let root = tree_root(device, directory);
        let (changes, read_at) = device
            .device
            .git_changes(&root)
            .await
            .map_err(ApiError::working_tree)?;
        let changes = WorkingTreeChanges { root, changes };
        Ok((changes, read_at))
    })
    .await?;
    Ok(read_answer(&read_at, Json(changes)))
}

/// The local branches of the working tree's repository and the one checked
/// out.
pub(super) async fn branches(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Path(id): Path<String>,
    QueryParams(ChangesQuery { directory }): QueryParams<ChangesQuery>,
) -> Result<Response, ApiError> {
    let (branches, read_at) = on_device(&state, &user.id, &id, None, async move |device| {
        device.device
            .git_branches(&tree_root(device, directory))
            .await
            .map_err(ApiError::history)
    })
    .await?;
    Ok(read_answer(&read_at, Json(branches)))
}

/// The repositories the conversation's execution directory is in or holds,
/// and those holding the directories the request names.
pub(super) async fn repositories(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Path(id): Path<String>,
    JsonBody(RepositoriesRequest { paths }): JsonBody<RepositoriesRequest>,
) -> Result<Response, ApiError> {
    let (repositories, read_at) = on_device(&state, &user.id, &id, None, async move |device| {
        device.device
            .git_repositories(&device.root, paths)
            .await
            .map_err(ApiError::history)
    })
    .await?;
    Ok(read_answer(&read_at, Json(repositories)))
}

/// One page of a branch's first-parent history.
pub(super) async fn log(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Path(id): Path<String>,
    QueryParams(LogQuery { directory, branch, after }): QueryParams<LogQuery>,
) -> Result<Response, ApiError> {
    let (log, read_at) = on_device(&state, &user.id, &id, None, async move |device| {
        device.device
            .git_log(
                &tree_root(device, directory),
                branch.as_ref().map(|branch| branch.as_str()),
                after.as_ref().map(CommitName::as_str),
            )
            .await
            .map_err(ApiError::history)
    })
    .await?;
    Ok(read_answer(&read_at, Json(log)))
}

/// One commit: its files against its first parent and the branches it is on.
pub(super) async fn commit(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Path(id): Path<String>,
    QueryParams(CommitQuery { directory, commit }): QueryParams<CommitQuery>,
) -> Result<Response, ApiError> {
    let (commit, read_at) = on_device(&state, &user.id, &id, None, async move |device| {
        device.device
            .git_commit(&tree_root(device, directory), commit.as_str())
            .await
            .map_err(ApiError::history)
    })
    .await?;
    Ok(read_answer(&read_at, Json(commit)))
}

/// A commit's file's two sides as text: its first parent's at `before`, the
/// earlier path of a file it renamed, empty for a file the commit added or
/// a root commit, and the commit's at `path`, empty for a file it deleted.
async fn commit_sides(
    device: &ConversationDevice,
    root: &str,
    before: &str,
    path: &str,
    commit: &str,
) -> Result<(ChangeSides, Option<ReadAt>), ApiError> {
    // The page after the commit starts with its first parent.
    let (log, log_at) = device
        .device
        .git_log(root, None, Some(commit))
        .await
        .map_err(ApiError::history)?;
    let parent = log.commits.first().map(|parent| parent.id.clone());
    let (before, after) = tokio::join!(
        async {
            match &parent {
                Some(parent) => Some(device.device.git_show(root, before, Some(parent)).await),
                None => None,
            }
        },
        device.device.git_show(root, path, Some(commit)),
    );
    let side = |read: Result<(bytes::Bytes, ReadAt), DeviceError>| match read {
        Ok((bytes, _)) => text_of(Vec::from(bytes)).map_err(ApiError::from),
        Err(error) if error.code() == Some("ENOENT") => Ok(String::new()),
        Err(error) => Err(ApiError::working_tree(error)),
    };
    let read_at = [Some(&log_at), before.as_ref().and_then(|read| read.as_ref().ok().map(|(_, at)| at)), after.as_ref().ok().map(|(_, at)| at)]
        .into_iter()
        .flatten()
        .cloned()
        .reduce(ReadAt::earlier);
    let answered = |error: ApiError| match &read_at {
        Some(read_at) => error.with_read_at(read_at.clone()),
        None => error,
    };
    let original = match before {
        Some(read) => side(read).map_err(answered)?,
        None => String::new(),
    };
    let modified = side(after).map_err(answered)?;
    Ok((ChangeSides { original, modified }, read_at))
}

/// One changed file's two sides as text: the last commit's, at `from` for a
/// renamed file and empty for a new one, and the working tree's, empty for
/// a deleted one; or with `commit`, that commit's change of it.
pub(super) async fn changed_file(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Path(id): Path<String>,
    QueryParams(TreeFileQuery { path, from, directory, commit }): QueryParams<TreeFileQuery>,
) -> Result<Response, ApiError> {
    let (sides, read_at) = on_device(&state, &user.id, &id, None, async move |device| {
        // A renamed file's old side is its content at its earlier path.
        let before = from.as_ref().unwrap_or(&path).as_str();
        if let Some(commit) = commit {
            return commit_sides(device, &tree_root(device, directory), before, path.as_str(), commit.as_str()).await;
        }
        // The two sides do not wait for each other (`crates/runner-device/src/lib.rs`).
        let root = tree_root(device, directory);
        let working = format!("{root}/{}", path.as_str());
        let (committed, working) = tokio::join!(
            device.device.git_show(&root, before, None),
            read_working_side(&device.device, &working),
        );
        // The answer holds what both reads hold from the earlier one on.
        let committed_at = match &committed {
            Ok((_, read_at)) => Some(read_at.clone()),
            Err(error) => error.read_at.clone(),
        };
        let working_at = match &working {
            Ok((_, read_at)) => Some(read_at.clone()),
            Err(error) => error.read_at.clone(),
        };
        let read_at = committed_at.into_iter().chain(working_at).reduce(ReadAt::earlier);
        let answered = |error: ApiError| match &read_at {
            Some(read_at) => error.with_read_at(read_at.clone()),
            None => error,
        };
        let original = match committed {
            Ok((bytes, _)) => text_of(Vec::from(bytes)).map_err(|refusal| answered(refusal.into()))?,
            Err(error) if error.code() == Some("ENOENT") => String::new(),
            Err(error) => return Err(answered(ApiError::working_tree(error))),
        };
        let modified = match working {
            Ok((text, _)) => text.map_err(|refusal| answered(refusal.into()))?,
            Err(error) if error.code() == Some("ENOENT") => String::new(),
            Err(error) => return Err(answered(ApiError::device_operation(error))),
        };
        Ok((ChangeSides { original, modified }, read_at))
    })
    .await?;
    let answer = Json(sides).into_response();
    Ok(match read_at {
        Some(read_at) => read_answer(&read_at, answer),
        None => answer,
    })
}

/// The working tree's side of a changed file as text, or why it is not
/// text, with the device's number before it read.
async fn read_working_side(
    device: &demi_runner_client::RemoteDevice,
    path: &str,
) -> Result<(Result<String, TextRefusal>, ReadAt), DeviceError> {
    let opened = device.read_pipe(path, TEXT_RANGE).await?;
    match text_of_stream(byte_stream(opened.body)).await {
        Ok(text) => Ok((Ok(text), opened.read_at)),
        Err(TextError::Refused(refusal)) => Ok((Err(refusal), opened.read_at)),
        Err(TextError::Device(error)) => Err(error.with_read_at(opened.read_at)),
    }
}

/// A changed file's last committed bytes, or a commit's, by range. Git's
/// copy is read whole on the runner and is at most 8 MiB, so it is held here
/// and served whole.
pub(super) async fn committed(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    method: Method,
    headers: HeaderMap,
    Path(id): Path<String>,
    QueryParams(CommittedFileQuery { path, directory, commit, download }): QueryParams<CommittedFileQuery>,
) -> Result<Response, ApiError> {
    let tree_path = path.as_str().to_owned();
    let (bytes, read_at) = on_device(&state, &user.id, &id, None, async move |device| {
        device.device
            .git_show(&tree_root(device, directory), &tree_path, commit.as_ref().map(CommitName::as_str))
            .await
            // A commit the repository lacks is not found, as the history says it.
            .map_err(if commit.is_some() { ApiError::history } else { ApiError::working_tree })
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
        Some(body) if method != Method::HEAD => Ok(read_answer(&read_at, (part.status(), answer, body))),
        _ => Ok(read_answer(&read_at, (part.status(), answer))),
    }
}

/// Runs `operation` on the conversation's device through its device access: the
/// primary device, or the bound device `device` names. The conversation's file
/// gate is let go once the device is resolved, so the call holds nothing a
/// transition waits behind.
async fn on_device<T, F>(
    state: &AppState,
    user: &UserId,
    id: &str,
    device: Option<&str>,
    operation: F,
) -> Result<T, ApiError>
where
    T: Send + 'static,
    F: AsyncFnOnce(&ConversationDevice) -> Result<T, ApiError> + Send + 'static,
{
    let conversation = conversation_id(id)?;
    let device = device
        .map(|device| {
            DeviceId::try_from(device)
                .map_err(|_| ApiError::from(DeviceAccessError::from(Refusal::NotAttached)))
        })
        .transpose()?;
    state
        .shards
        .of(user)
        .call(move |shard, cancel| async move {
            shard
                .device_shard()
                .with_unheld_device(&conversation, device.as_ref(), &cancel, operation)
                .await?
        })
        .await?
}

/// The path's conversation id; one that is not an id names no conversation.
pub(super) fn conversation_id(id: &str) -> Result<ConversationId, ApiError> {
    ConversationId::try_from(id).map_err(|_| DeviceAccessError::Missing.into())
}

pub(super) fn header(headers: &HeaderMap, name: HeaderName) -> Option<String> {
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
///
/// They are also `no-transform` (RFC 9110 § 7.7): the page reads a file's
/// size from a `HEAD`'s `Content-Length` and asks for byte ranges of the
/// file itself, and a proxy that compresses the answer, as a CDN does to an
/// SVG, drops that length and changes what the bytes are. The directive is
/// the one standard way to keep every proxy from doing so.
fn raw_file_headers(versioned: bool) -> HeaderMap {
    let mut headers = HeaderMap::new();
    let cache = if versioned {
        "private, max-age=31536000, immutable, no-transform"
    } else {
        "private, no-cache, no-transform"
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
    /// How its bytes may be shown.
    content: &'a HeaderMap,
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
        headers.extend(self.content.clone());
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
