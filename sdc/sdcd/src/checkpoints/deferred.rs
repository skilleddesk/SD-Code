//! Checkpoints for changes made from the window, taken after the answer instead of before it.
//!
//! Save, Rename and Delete each took a checkpoint of the whole folder before touching one file, and the answer waited
//! for it: git walks every file of the folder, and on a slow disk or a large project that was seconds in which the
//! sidebar did nothing. The checkpoint's *content* does not need that wait. What a checkpoint must hold is the files as
//! they were before the change, and only the touched paths can differ from what is on disk a moment later - so the
//! caller captures those paths' pre-image (a few files, a moment) and answers; this worker then reads the folder once
//! and puts the pre-images back over what it finds (`git::checkpoint_with`). The result is the same commit the old
//! path made, and a checkpoint taken for several changes in a row is one walk instead of one each.
//!
//! One worker thread per folder, started on first use. Changes that arrive while it is busy, or within the short batching
//! pause, share a checkpoint; each still gets its own row (and its own `CheckpointSaved`), so the Time Machine lists
//! every change by name.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use super::Fresh;
use crate::git::PreImage;
use crate::store::sqlite::Store;

/// How long the worker waits for more changes before it walks the folder.
const BATCH: Duration = Duration::from_millis(150);

/// One change waiting for its checkpoint.
pub struct Pending {
    pub store: Arc<Store>,
    pub session_id: String,
    pub turn: i64,
    pub title: String,
    pub root: PathBuf,
    pub image: PreImage,
    /// Called with the finished checkpoint - the daemon pushes `CheckpointSaved` from it.
    pub done: Box<dyn FnOnce(Fresh) + Send>,
}

static QUEUES: OnceLock<Mutex<HashMap<PathBuf, Sender<Pending>>>> = OnceLock::new();
static WAITING: AtomicUsize = AtomicUsize::new(0);

/// Queues a change. Returns at once.
pub fn submit(pending: Pending) {
    let queues = QUEUES.get_or_init(|| Mutex::new(HashMap::new()));
    let mut queues = queues.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let root = pending.root.clone();
    let mut pending = Some(pending);

    WAITING.fetch_add(1, Ordering::SeqCst);

    /* A worker whose thread ended (it never does, short of a panic) is replaced rather than trusted. */
    if let Some(queue) = queues.get(&root) {
        match queue.send(pending.take().expect("pending is set")) {
            Ok(()) => return,
            Err(returned) => pending = Some(returned.0),
        }
    }

    let (sender, receiver) = channel();
    let work = root.clone();

    std::thread::Builder::new()
        .name("sdcd-checkpoints".into())
        .spawn(move || work_through(&work, receiver))
        .ok();

    if let Some(pending) = pending {
        let _ = sender.send(pending);
    }

    queues.insert(root, sender);
}

/// Waits (up to `limit`) for every queued checkpoint to be written - called as the daemon stops, so a Save made in its
/// last second is still undoable after the restart.
pub fn drain(limit: Duration) {
    let started = Instant::now();

    while WAITING.load(Ordering::SeqCst) > 0 && started.elapsed() < limit {
        std::thread::sleep(Duration::from_millis(25));
    }
}

fn work_through(root: &std::path::Path, receiver: Receiver<Pending>) {
    while let Ok(first) = receiver.recv() {
        std::thread::sleep(BATCH);

        let mut batch = vec![first];

        while let Ok(more) = receiver.try_recv() {
            batch.push(more);
        }

        let count = batch.len();
        let title = if count == 1 { batch[0].title.clone() } else { format!("{} (+{} more)", batch[0].title, count - 1) };
        let images: Vec<PreImage> = batch.iter_mut().map(|pending| std::mem::replace(&mut pending.image, PreImage { paths: Vec::new(), entries: Vec::new() })).collect();
        let turn = batch[0].turn;

        match crate::git::checkpoint_with(root, &format!("sdcd: turn {turn} — {title}"), &images) {
            Ok(files_hash) => {
                for pending in batch {
                    let id = format!("cp-{}-{}", pending.session_id, pending.turn);

                    if pending
                        .store
                        .insert_checkpoint(&id, &pending.session_id, pending.turn, &pending.title, None, &files_hash)
                        .is_ok()
                    {
                        (pending.done)(Fresh { id, turn: pending.turn, title: pending.title, thumbnail: None, files_hash: files_hash.clone() });
                    }

                    WAITING.fetch_sub(1, Ordering::SeqCst);
                }
            }
            Err(error) => {
                eprintln!("sdcd: a checkpoint of {} could not be taken: {}", root.display(), error.message);
                WAITING.fetch_sub(count, Ordering::SeqCst);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A Save answers before its checkpoint is written; the checkpoint that arrives holds the file as it was.
    #[test]
    fn the_checkpoint_arrives_later_and_holds_the_file_as_it_was() {
        let project = tempfile::tempdir().unwrap();
        let file = project.path().join("notes.md");

        std::fs::write(&file, "one\n").unwrap();

        let store = Arc::new(Store::in_memory().unwrap());

        store.upsert_host("local", "Local", "local", None, "connected", None).unwrap();
        store.insert_session("s1", "local", "Notes", "add", None).unwrap();

        let image = crate::git::preimage(project.path(), &[file.as_path()]).unwrap();

        std::fs::write(&file, "one\ntwo\n").unwrap();

        let (sender, receiver) = std::sync::mpsc::channel();

        submit(Pending {
            store: store.clone(),
            session_id: "s1".into(),
            turn: 7,
            title: "Before editing notes.md".into(),
            root: project.path().to_path_buf(),
            image,
            done: Box::new(move |fresh| {
                let _ = sender.send(fresh);
            }),
        });

        let fresh = receiver.recv_timeout(Duration::from_secs(30)).expect("the checkpoint is written in the background");

        assert_eq!(fresh.id, "cp-s1-7");
        assert_eq!(fresh.title, "Before editing notes.md");

        let shadow = crate::git::Shadow::of(project.path()).unwrap();
        let held = shadow.run(&["show", &format!("{}:notes.md", fresh.files_hash)]).unwrap();

        assert_eq!(held, "one\n", "the checkpoint is of the file before the save");
        assert_eq!(store.checkpoints("s1").unwrap().len(), 1, "and it is in the Time Machine's list");

        drain(Duration::from_secs(5));
    }

    /// Each change keeps its own row, and each row's checkpoint holds that change's file as it was before it. (When both
    /// arrive within one batch they share a walk of the folder; when the second comes later it is its own checkpoint, of
    /// the folder as it stood just before *that* change - so the first change's result is rightly in it.)
    #[test]
    fn every_change_keeps_its_own_row_and_its_own_file_as_it_was() {
        let project = tempfile::tempdir().unwrap();
        let (a, b) = (project.path().join("a.txt"), project.path().join("b.txt"));

        std::fs::write(&a, "a1\n").unwrap();
        std::fs::write(&b, "b1\n").unwrap();

        let store = Arc::new(Store::in_memory().unwrap());

        store.upsert_host("local", "Local", "local", None, "connected", None).unwrap();
        store.insert_session("s2", "local", "Two", "add", None).unwrap();

        let (sender, receiver) = std::sync::mpsc::channel();

        for (turn, path, text) in [(1_i64, &a, "a2\n"), (2, &b, "b2\n")] {
            let image = crate::git::preimage(project.path(), &[path.as_path()]).unwrap();

            std::fs::write(path, text).unwrap();

            let sender = sender.clone();

            submit(Pending {
                store: store.clone(),
                session_id: "s2".into(),
                turn,
                title: format!("Before editing {}", path.file_name().unwrap().to_string_lossy()),
                root: project.path().to_path_buf(),
                image,
                done: Box::new(move |fresh| {
                    let _ = sender.send(fresh);
                }),
            });
        }

        let first = receiver.recv_timeout(Duration::from_secs(30)).unwrap();
        let second = receiver.recv_timeout(Duration::from_secs(30)).unwrap();

        assert_ne!(first.id, second.id);
        assert_eq!(store.checkpoints("s2").unwrap().len(), 2);

        let shadow = crate::git::Shadow::of(project.path()).unwrap();

        let (one, two) = if first.turn == 1 { (&first, &second) } else { (&second, &first) };

        assert_eq!(shadow.run(&["show", &format!("{}:a.txt", one.files_hash)]).unwrap(), "a1\n");
        assert_eq!(shadow.run(&["show", &format!("{}:b.txt", two.files_hash)]).unwrap(), "b1\n");

        drain(Duration::from_secs(5));
    }
}
