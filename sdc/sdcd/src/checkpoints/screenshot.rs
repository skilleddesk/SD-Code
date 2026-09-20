//! Checkpoint screenshots (master spec section 14.1).
//!
//! A checkpoint's thumbnail is a picture of the preview at that moment. Taking one needs a rendering
//! engine attached to the preview pane, which is the app's side of the boundary - the WebView the
//! user is looking at is the only thing that can photograph it.
//!
//! So this module owns the *contract* and the *path*, not the pixels:
//!
//! * `thumbnail_path(id)` is where a screenshot lives, so the app's preview can write one and the
//!   checkpoint row can name it without either side guessing;
//! * `capture()` returns `None` and says why, which is what the Time Machine tab renders as its
//!   gradient placeholder rather than a broken image.
//!
//! The rewind does not depend on it: `rewind/` restores the *files* from the shadow repository, and
//! those are captured for real today.

use std::path::PathBuf;

use crate::sdcp::envelope::ErrorObject;

/// Where the screenshot of a checkpoint lives, whether or not it has been taken yet.
pub fn thumbnail_path(checkpoint_id: &str) -> Result<PathBuf, ErrorObject> {
    let dir = crate::paths::checkpoints_dir().map_err(ErrorObject::internal)?;

    Ok(dir.join(format!("{checkpoint_id}.png")))
}

/// True when a screenshot exists for this checkpoint.
pub fn has_thumbnail(checkpoint_id: &str) -> bool {
    thumbnail_path(checkpoint_id).map(|path| path.exists()).unwrap_or(false)
}

/// Records a screenshot the *app* wrote into the checkpoints directory, and returns its path.
///
/// This is the half of the contract the daemon can keep: the app knows the bytes (it can photograph
/// its own preview), the daemon knows where they belong. A file dropped here is what the Time Machine
/// tab shows.
pub fn record(checkpoint_id: &str, bytes: &[u8]) -> Result<String, ErrorObject> {
    let path = thumbnail_path(checkpoint_id)?;

    std::fs::write(&path, bytes).map_err(ErrorObject::internal)?;

    Ok(path.display().to_string())
}

/// Why there is no screenshot of this checkpoint, in the words the Time Machine tab can show.
pub fn capture() -> Option<String> {
    None
}

/// The reason `capture()` returns `None`, for the doctor and for the UI's tooltip.
pub const CAPTURE_REASON: &str =
    "the preview screenshot is taken by the app's WebView; the daemon records it when it arrives";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_thumbnail_path_is_inside_the_checkpoints_directory() {
        let path = thumbnail_path("cp-s1-14").unwrap();

        assert!(path.ends_with("cp-s1-14.png"));
        assert!(path.starts_with(crate::paths::checkpoints_dir().unwrap()));
    }

    #[test]
    fn recording_bytes_makes_the_thumbnail_exist() {
        let id = "cp-test-screenshot";
        let path = record(id, b"\x89PNG\r\n\x1a\n not really a png").unwrap();

        assert!(has_thumbnail(id));
        assert!(std::path::Path::new(&path).exists());

        std::fs::remove_file(path).ok();
        assert!(!has_thumbnail(id));
    }

    #[test]
    fn capture_explains_itself_instead_of_failing() {
        assert!(capture().is_none());
        assert!(!CAPTURE_REASON.is_empty());
    }
}
