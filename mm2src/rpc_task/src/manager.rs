use crate::task::RpcTaskTypes;
use crate::{AtomicTaskId, FinishedTaskResult, RpcTask, RpcTaskError, RpcTaskHandle, RpcTaskResult, RpcTaskStatus,
            RpcTaskStatusAlias, TaskAbortHandle, TaskAbortHandler, TaskId, TaskStatus, TaskStatusError,
            UserActionSender};
use common::executor::spawn;
use common::log::{debug, warn};
use common::mm_error::prelude::*;
use futures::channel::oneshot;
use futures::future::{select, Either};
use std::collections::hash_map::Entry;
use std::collections::HashMap;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex, Weak};

pub type RpcTaskManagerShared<Task> = Arc<Mutex<RpcTaskManager<Task>>>;
pub(crate) type RpcTaskManagerWeak<Task> = Weak<Mutex<RpcTaskManager<Task>>>;

static NEXT_RPC_TASK_ID: AtomicTaskId = AtomicTaskId::new(0);

fn next_rpc_task_id() -> TaskId { NEXT_RPC_TASK_ID.fetch_add(1, Ordering::Relaxed) }

pub struct RpcTaskManager<Task: RpcTask> {
    tasks: HashMap<TaskId, TaskStatusExt<Task>>,
}

impl<Task: RpcTask> Default for RpcTaskManager<Task> {
    fn default() -> Self { RpcTaskManager { tasks: HashMap::new() } }
}

impl<Task: RpcTask> RpcTaskManager<Task> {
    /// Create new instance of `RpcTaskHandle` attached to the only one `RpcTask`.
    /// This function registers corresponding RPC task in the `RpcTaskManager` and returns the task id.
    pub fn spawn_rpc_task(this: &RpcTaskManagerShared<Task>, task: Task) -> RpcTaskResult<TaskId> {
        let initial_task_status = task.initial_status();
        let (task_id, task_abort_handler) = {
            let mut task_manager = this
                .lock()
                .map_to_mm(|e| RpcTaskError::Internal(format!("RpcTaskManager is not available: {}", e)))?;
            task_manager.register_task(initial_task_status)?
        };
        let task_handle = RpcTaskHandle {
            task_manager: RpcTaskManagerShared::downgrade(this),
            task_id,
        };

        let fut = async move {
            debug!("Spawn RPC task '{}'", task_id);
            let task_fut = task.run(&task_handle);
            let task_result = match select(task_fut, task_abort_handler).await {
                // The task has finished.
                Either::Left((task_result, _abort_handler)) => Some(task_result),
                // The task has been aborted from outside.
                Either::Right((_aborted, _task)) => None,
            };
            // We can't finish or abort the task in the match statement above since `task_handle` is borrowed here:
            // `task.run(&task_handle)`.
            match task_result {
                Some(task_result) => {
                    debug!("RPC task '{}' has been finished", task_id);
                    task_handle.finish(task_result);
                },
                None => {
                    debug!("RPC task '{}' has been aborted", task_id);
                    task_handle.abort();
                },
            }
        };
        spawn(fut);
        Ok(task_id)
    }

    /// Returns a task status if it exists, otherwise returns `None`.
    pub fn task_status(&mut self, task_id: TaskId, forget_if_ready: bool) -> Option<RpcTaskStatusAlias<Task>> {
        let entry = match self.tasks.entry(task_id) {
            Entry::Occupied(entry) => entry,
            Entry::Vacant(_) => return None,
        };
        let rpc_status = match entry.get() {
            TaskStatusExt::InProgress { status, .. } => RpcTaskStatus::InProgress(status.clone()),
            TaskStatusExt::Awaiting { status, .. } => RpcTaskStatus::UserActionRequired(status.clone()),
            TaskStatusExt::Ready(ready) => {
                // I prefer cloning `ready` instead of removing `TaskStatusX` and matching/unwrapping it again.
                let rpc_status = RpcTaskStatus::Ready(ready.clone());
                if forget_if_ready {
                    entry.remove();
                }
                rpc_status
            },
        };
        Some(rpc_status)
    }

    pub fn new_shared() -> RpcTaskManagerShared<Task> { Arc::new(Mutex::new(Self::default())) }

    pub fn contains(&self, task_id: TaskId) -> bool { self.tasks.contains_key(&task_id) }

    /// Cancel task if it's in progress.
    pub fn cancel_task(&mut self, task_id: TaskId) -> RpcTaskResult<()> {
        self.tasks
            .remove(&task_id)
            .map(|_| ())
            .or_mm_err(|| self.rpc_task_error_if_not_found(task_id, TaskStatusError::InProgress))
    }

    pub(crate) fn register_task(
        &mut self,
        task_initial_in_progress_status: Task::InProgressStatus,
    ) -> RpcTaskResult<(TaskId, TaskAbortHandler)> {
        let task_id = next_rpc_task_id();
        let (abort_handle, abort_handler) = oneshot::channel();
        match self.tasks.entry(task_id) {
            Entry::Occupied(_entry) => MmError::err(RpcTaskError::UnexpectedTaskStatus {
                task_id,
                actual: TaskStatusError::InProgress,
                expected: TaskStatusError::Idle,
            }),
            Entry::Vacant(entry) => {
                entry.insert(TaskStatusExt::InProgress {
                    status: task_initial_in_progress_status,
                    abort_handle,
                });
                Ok((task_id, abort_handler))
            },
        }
    }

    pub(crate) fn update_task_status(&mut self, task_id: TaskId, status: TaskStatus<Task>) -> RpcTaskResult<()> {
        match status {
            TaskStatus::Ready(result) => self.on_task_finished(task_id, result),
            TaskStatus::InProgress(in_progress) => self.update_in_progress_status(task_id, in_progress),
            TaskStatus::UserActionRequired {
                awaiting_status,
                user_action_tx,
            } => self.set_task_is_waiting_for_user_action(task_id, awaiting_status, user_action_tx),
        }
    }

    fn rpc_task_error_if_not_found(&self, task_id: TaskId, expected: TaskStatusError) -> RpcTaskError {
        let actual = match self.tasks.get(&task_id) {
            Some(TaskStatusExt::InProgress { .. }) => TaskStatusError::InProgress,
            Some(TaskStatusExt::Awaiting { .. }) => TaskStatusError::AwaitingUserAction,
            Some(TaskStatusExt::Ready(_)) => TaskStatusError::Finished,
            None => return RpcTaskError::NoSuchTask(task_id),
        };
        RpcTaskError::UnexpectedTaskStatus {
            task_id,
            actual,
            expected,
        }
    }

    fn on_task_finished(
        &mut self,
        task_id: TaskId,
        task_result: FinishedTaskResult<Task::Item, Task::Error>,
    ) -> RpcTaskResult<()> {
        if self.tasks.insert(task_id, TaskStatusExt::Ready(task_result)).is_none() {
            warn!("Finished task '{}' was not ongoing", task_id);
        }
        Ok(())
    }

    fn update_in_progress_status(&mut self, task_id: TaskId, status: Task::InProgressStatus) -> RpcTaskResult<()> {
        match self.tasks.remove(&task_id) {
            Some(TaskStatusExt::InProgress { abort_handle, .. })
            | Some(TaskStatusExt::Awaiting { abort_handle, .. }) => {
                // Insert new in-progress status to the tasks container.
                self.tasks
                    .insert(task_id, TaskStatusExt::InProgress { status, abort_handle });
                Ok(())
            },
            Some(ready @ TaskStatusExt::Ready(_)) => {
                // Return the task result to the tasks container.
                self.tasks.insert(task_id, ready);
                MmError::err(RpcTaskError::UnexpectedTaskStatus {
                    task_id,
                    actual: TaskStatusError::Finished,
                    expected: TaskStatusError::InProgress,
                })
            },
            None => MmError::err(RpcTaskError::NoSuchTask(task_id)),
        }
    }

    fn set_task_is_waiting_for_user_action(
        &mut self,
        task_id: TaskId,
        status: Task::AwaitingStatus,
        action_sender: UserActionSender<Task::UserAction>,
    ) -> RpcTaskResult<()> {
        match self.tasks.remove(&task_id) {
            Some(TaskStatusExt::InProgress {
                status: next_in_progress_status,
                abort_handle,
            }) => {
                // Insert new awaiting status to the tasks container.
                self.tasks.insert(task_id, TaskStatusExt::Awaiting {
                    status,
                    abort_handle,
                    action_sender,
                    next_in_progress_status,
                });
                Ok(())
            },
            Some(unexpected) => {
                // Return the status to the tasks container.
                self.tasks.insert(task_id, unexpected);
                MmError::err(self.rpc_task_error_if_not_found(task_id, TaskStatusError::InProgress))
            },
            None => MmError::err(RpcTaskError::NoSuchTask(task_id)),
        }
    }

    /// Notify a spawned interrupted RPC task about the user action if it await the action.
    pub fn on_user_action(&mut self, task_id: TaskId, user_action: Task::UserAction) -> RpcTaskResult<()> {
        match self.tasks.remove(&task_id) {
            Some(TaskStatusExt::Awaiting {
                action_sender,
                abort_handle,
                next_in_progress_status: status,
                ..
            }) => {
                let result = action_sender
                    .send(user_action)
                    // The task seems to be canceled/aborted for some reason.
                    .map_to_mm(|_user_action| RpcTaskError::Canceled);
                // Insert new in-progress status to the tasks container.
                self.tasks
                    .insert(task_id, TaskStatusExt::InProgress { status, abort_handle });
                result
            },
            Some(unexpected) => {
                // Return the unexpected status to the tasks container.
                self.tasks.insert(task_id, unexpected);
                MmError::err(self.rpc_task_error_if_not_found(task_id, TaskStatusError::AwaitingUserAction))
            },
            None => MmError::err(RpcTaskError::NoSuchTask(task_id)),
        }
    }
}

/// `TaskStatus` extended with `TaskAbortHandle`.
/// This is stored in the [`RpcTaskManager::tasks`] container.
enum TaskStatusExt<Task: RpcTaskTypes> {
    InProgress {
        status: Task::InProgressStatus,
        abort_handle: TaskAbortHandle,
    },
    Awaiting {
        status: Task::AwaitingStatus,
        action_sender: UserActionSender<Task::UserAction>,
        abort_handle: TaskAbortHandle,
        next_in_progress_status: Task::InProgressStatus,
    },
    Ready(FinishedTaskResult<Task::Item, Task::Error>),
}
