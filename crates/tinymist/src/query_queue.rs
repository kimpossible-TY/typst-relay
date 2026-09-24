//! Admission for synchronous semantic analysis.

#[cfg(feature = "system")]
use std::sync::atomic::Ordering;
use std::sync::Arc;

use sync_ls::{internal_error, ErrorCode, LspResult, ResponseError};
use tokio::sync::{watch, Semaphore};

use crate::project::{LspInterrupt, ProjectInsId};

/// Keeps obsolete requests out of the synchronous analysis workers.
#[derive(Clone)]
pub(crate) struct QueryQueue {
    permit: Arc<Semaphore>,
    revision: watch::Sender<u64>,
}

impl Default for QueryQueue {
    fn default() -> Self {
        Self {
            permit: Arc::new(Semaphore::new(1)),
            revision: watch::channel(0).0,
        }
    }
}

impl QueryQueue {
    pub(crate) fn revision(&self) -> u64 {
        *self.revision.borrow()
    }

    pub(crate) fn invalidate(&self) {
        self.revision
            .send_modify(|revision| *revision = revision.wrapping_add(1));
    }

    /// Invalidates snapshots when a compiler event changes their inputs.
    pub(crate) fn invalidate_for_interrupt(&self, interrupt: &LspInterrupt) {
        let changes_inputs = match interrupt {
            LspInterrupt::Memory(_)
            | LspInterrupt::Fs(_)
            | LspInterrupt::Font(_)
            | LspInterrupt::CreationTimestamp(_)
            | LspInterrupt::Save(_) => true,
            // Semantic queries use the primary project. Changes to a dedicated
            // preview's task do not change those snapshots.
            LspInterrupt::ChangeTask(id, _) => id == &ProjectInsId::PRIMARY,
            // Compilation produces a result from existing inputs. Settling
            // only removes dedicated projects; it cannot remove the primary.
            LspInterrupt::Compile(_) | LspInterrupt::Compiled(_) | LspInterrupt::Settle(_) => false,
        };
        if changes_inputs {
            self.invalidate();
        }
    }

    fn check(&self, revision: u64) -> LspResult<()> {
        if self.revision() != revision {
            return Err(ResponseError {
                code: ErrorCode::ContentModified as i32,
                message: "document changed before analysis started".into(),
                data: None,
            });
        }
        Ok(())
    }

    pub(crate) async fn run<T: Send + 'static>(
        self,
        revision: u64,
        work: impl FnOnce() -> LspResult<T> + Send + 'static,
    ) -> LspResult<T> {
        let queued_at = tinymist_std::time::Instant::now();
        let mut revisions = self.revision.subscribe();
        self.check(revision)?;
        let permit = tokio::select! {
            biased;
            _ = revisions.changed() => {
                self.check(revision)?;
                unreachable!("revision notifications always advance the revision");
            }
            permit = self.permit.clone().acquire_owned() => permit.map_err(internal_error)?,
        };
        self.check(revision)?;
        log::debug!(
            "QueryQueue: admitted revision {revision} after {:?}",
            queued_at.elapsed()
        );

        #[cfg(feature = "system")]
        {
            // Aborting the outer future must not release the permit while its
            // synchronous worker is still using the shared analysis caches.
            // If it is still queued in the blocking pool, skip it on dispatch.
            use std::sync::atomic::AtomicBool;
            struct Active(Arc<AtomicBool>);
            impl Drop for Active {
                fn drop(&mut self) {
                    self.0.store(false, Ordering::Release);
                }
            }
            let active = Active(Arc::new(AtomicBool::new(true)));
            let running = active.0.clone();
            let result = tokio::task::spawn_blocking(move || {
                let _permit = permit;
                self.check(revision)?;
                if !running.load(Ordering::Acquire) {
                    return Err(ResponseError {
                        code: ErrorCode::RequestCanceled as i32,
                        message: "analysis cancelled before execution".into(),
                        data: None,
                    });
                }
                work()
            })
            .await
            .map_err(internal_error)?;
            drop(active);
            result
        }
        #[cfg(not(feature = "system"))]
        {
            let _permit = permit;
            work()
        }
    }
}

#[cfg(all(test, feature = "system"))]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicBool;

    use crate::project::{CompileOnceArgs, CompiledArtifact, LspUniverseBuilder, WorldProvider};
    use crate::world::base::{CompileSnapshot, WorldComputeGraph};
    use crate::world::vfs::{notify::MemoryEvent, FileChangeSet, FilesystemEvent};
    use crate::world::TaskInputs;
    use clap::Parser;

    #[test]
    fn input_interrupts_invalidate_primary_snapshot() {
        let fonts = Arc::new(LspUniverseBuilder::only_embedded_fonts().unwrap());
        let queue = QueryQueue::default();
        for interrupt in [
            LspInterrupt::Memory(MemoryEvent::Sync(FileChangeSet::default())),
            LspInterrupt::Fs(FilesystemEvent::Update(FileChangeSet::default(), false)),
            LspInterrupt::Font(fonts),
            LspInterrupt::CreationTimestamp(Some(123)),
            LspInterrupt::Save(std::path::Path::new("/main.typ").into()),
            LspInterrupt::ChangeTask(ProjectInsId::PRIMARY, TaskInputs::default()),
        ] {
            let before = queue.revision();
            queue.invalidate_for_interrupt(&interrupt);
            assert_ne!(queue.revision(), before, "{interrupt:?}");
        }
        let before = queue.revision();
        queue.invalidate_for_interrupt(&LspInterrupt::ChangeTask(
            ProjectInsId("preview".into()),
            TaskInputs::default(),
        ));
        assert_eq!(queue.revision(), before);
    }

    #[tokio::test]
    async fn preview_memory_event_releases_waiting_snapshot() {
        let queue = QueryQueue::default();
        let permit = queue.permit.clone().acquire_owned().await.unwrap();
        let snapshot = Arc::new(());
        let retained = Arc::downgrade(&snapshot);
        let executed = Arc::new(AtomicBool::new(false));
        let observed = executed.clone();
        let task = tokio::spawn(queue.clone().run(queue.revision(), move || {
            let _snapshot = snapshot;
            observed.store(true, Ordering::Release);
            Ok(())
        }));
        tokio::task::yield_now().await;
        assert!(retained.upgrade().is_some());

        // Preview editor updates arrive directly as compiler interrupts,
        // without passing through ServerState::update_sources.
        let files = FileChangeSet::new_inserts(vec![(
            std::path::Path::new("/preview/main.typ").into(),
            Ok(reflexo_typst::Bytes::from_string("changed".to_owned())).into(),
        )]);
        queue.invalidate_for_interrupt(&LspInterrupt::Memory(MemoryEvent::Update(files)));
        let error = tokio::time::timeout(std::time::Duration::from_secs(1), task)
            .await
            .unwrap()
            .unwrap()
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::ContentModified as i32);
        assert!(!executed.load(Ordering::Acquire));
        assert!(retained.upgrade().is_none());
        assert_eq!(queue.permit.available_permits(), 0);
        drop(permit);
    }

    #[tokio::test]
    async fn compile_results_keep_unchanged_waiting_query() {
        let temp = tempfile::tempdir().unwrap();
        let input = temp.path().join("main.typ");
        std::fs::write(&input, "#let n = 1").unwrap();
        let args = CompileOnceArgs::parse_from([
            "tinymist",
            "--ignore-system-fonts",
            input.to_str().unwrap(),
        ]);
        let verse = args.resolve().unwrap();
        let graph = WorldComputeGraph::new(CompileSnapshot::from_world(verse.snapshot()));
        let artifact = CompiledArtifact::from_graph(graph.clone(), false);
        assert!(artifact.doc.is_some());

        let queue = QueryQueue::default();
        let revision = queue.revision();
        let permit = queue.permit.clone().acquire_owned().await.unwrap();
        let task = tokio::spawn(
            queue
                .clone()
                .run(revision, move || Ok(graph.snap.world.revision())),
        );
        tokio::task::yield_now().await;
        queue.invalidate_for_interrupt(&LspInterrupt::Compile(ProjectInsId::PRIMARY));
        queue.invalidate_for_interrupt(&LspInterrupt::Compiled(artifact));
        queue.invalidate_for_interrupt(&LspInterrupt::Settle(ProjectInsId("preview".into())));
        assert_eq!(queue.revision(), revision);
        assert!(!task.is_finished());
        drop(permit);
        tokio::time::timeout(std::time::Duration::from_secs(1), task)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
    }

    #[tokio::test]
    async fn changed_snapshot_never_runs() {
        let queue = QueryQueue::default();
        let revision = queue.revision();
        let permit = queue.permit.clone().acquire_owned().await.unwrap();
        let executed = Arc::new(AtomicBool::new(false));
        let observed = executed.clone();
        let task = tokio::spawn(queue.clone().run(revision, move || {
            observed.store(true, Ordering::Release);
            Ok(())
        }));
        tokio::task::yield_now().await;
        queue.invalidate();
        // Invalidation must release obsolete snapshots even while another
        // query remains running and holds the only admission permit.
        let error = tokio::time::timeout(std::time::Duration::from_secs(1), task)
            .await
            .unwrap()
            .unwrap()
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::ContentModified as i32);
        assert!(!executed.load(Ordering::Acquire));
        drop(permit);
    }

    #[tokio::test]
    async fn cancelled_waiter_never_runs() {
        let queue = QueryQueue::default();
        let permit = queue.permit.clone().acquire_owned().await.unwrap();
        let executed = Arc::new(AtomicBool::new(false));
        let observed = executed.clone();
        let task = tokio::spawn(queue.clone().run(queue.revision(), move || {
            observed.store(true, Ordering::Release);
            Ok(())
        }));
        tokio::task::yield_now().await;
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        drop(permit);
        queue
            .clone()
            .run(queue.revision(), || Ok(()))
            .await
            .unwrap();
        assert!(!executed.load(Ordering::Acquire));
    }

    #[tokio::test]
    async fn cancelled_running_work_keeps_its_permit() {
        let queue = QueryQueue::default();
        let (started, ready) = tokio::sync::oneshot::channel();
        let (release, wait) = std::sync::mpsc::channel();
        let task = tokio::spawn(queue.clone().run(queue.revision(), move || {
            started.send(()).unwrap();
            wait.recv().unwrap();
            Ok(())
        }));
        ready.await.unwrap();
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        assert_eq!(queue.permit.available_permits(), 0);
        release.send(()).unwrap();
        queue
            .clone()
            .run(queue.revision(), || Ok(()))
            .await
            .unwrap();
        assert_eq!(queue.permit.available_permits(), 1);
    }
}
