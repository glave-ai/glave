//! The session API's methods for files: a file added to a message, one a thread holds or made,
//! the Library's and sharing's. Written apart from `request.rs` and joined to its one table:
//! [`with_file_methods`] takes the rest of the table and hands it, these after it, to
//! `methods!`, so `Request` stays one enum.

/// Hands `methods!` the table it is given, then the files' methods.
macro_rules! with_file_methods {
    ($($table:tt)*) => {
        methods! {
            $($table)*
        /// Keeps a file the person added to a message they are writing, in `project` (their Chats
        /// when absent), for `turn/start` to name: kept a day unless a message is sent with it.
        /// Images (PNG, JPEG, GIF, WebP), PDFs and text, each up to 10 MB, kept as what they hold
        /// says they are; anything else is refused, videos with a sentence of their own.
        "file/upload" => FileUpload {
            #[serde(default, skip_serializing_if = "Option::is_none")]
            project: Option<ProjectId>,
            /// Its name, as the person's computer called it.
            name: String,
            /// Its media type, such as `image/png`; `application/octet-stream` when unknown. What
            /// it holds decides whether it is an image or a PDF; this names a text file's kind.
            media_type: String,
            /// Its content, in base64.
            data: String,
        } -> [File],
        /// A piece of a file `thread` holds, by its content's SHA-256: one its own messages or tool
        /// results name, such as an image the agent made, and nothing else its project keeps. Read
        /// from `offset` (the start when absent) a piece at a time, each answer saying where the
        /// next starts.
        "file/read" => FileRead {
            thread: ThreadId,
            #[schemars(with = "String")]
            sha256: Sha256,
            #[serde(default, skip_serializing_if = "Option::is_none")]
            offset: Option<u64>,
        } -> [FilePiece],
        /// A file `thread` made as Markdown to preview it by, at most 256 KB with a sentence for what
        /// was left out; refused for a file a client draws or downloads itself (`session.md`).
        "file/preview" => FilePreview { thread: ThreadId, #[schemars(with = "String")] sha256: Sha256 }
            -> [FilePreview],
        /// What glave made for the caller and keeps beyond any chat: every project's, newest first.
        "library/list" => LibraryList {} -> [Library],
        /// A piece of a Library item's file, read as `file/read` reads one. The item is named with
        /// the project it was made in, which the caller must own.
        "library/read" => LibraryRead {
            project: ProjectId,
            asset: AssetId,
            #[serde(default)]
            offset: Option<u64>,
        } -> [FilePiece],
        /// Takes an item out of the caller's Library, letting go of its file first; the chat it was
        /// made in keeps its own copy.
        "library/remove" => LibraryRemove { project: ProjectId, asset: AssetId } -> [Done],
        /// A file's share, where the caller shares it.
        "share/get" => ShareGet { #[serde(flatten)] of: ShareOf } -> [Share, Done],
        /// Shares a file the caller made at a link of its own, changes who may open it, or stops it.
        "share/set" => ShareSet { #[serde(flatten)] set: ShareSetting } -> [Share, Done],
        }
    };
}

pub(super) use with_file_methods;
