//! The shell's calls that change the app, queued in the order they came and carried out in the
//! next frame (never inside the shell's call).

use super::mode::Mode;
use super::open::OpenRequest;

pub(crate) enum Request {
    Open(OpenRequest),
    /// v2 `setMode(docId, mode, canDownload)`.
    SetMode {
        doc_id: String,
        mode: Mode,
        can_download: bool,
    },
    /// v2 `setUser(user)`: the account's name, or `None` for the local preference.
    SetUser(Option<String>),
}

impl From<OpenRequest> for Request {
    fn from(request: OpenRequest) -> Self {
        Request::Open(request)
    }
}
