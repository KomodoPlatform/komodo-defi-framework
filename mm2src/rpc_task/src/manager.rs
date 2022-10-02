use crate::task::RpcTaskTypes;
use crate::{AtomicTaskId, RpcTask, RpcTaskError, RpcTaskHandle, RpcTaskResult, RpcTaskStatus, RpcTaskStatusAlias,
            TaskAbortHandle, TaskAbortHandler, TaskId, TaskStatus, TaskStatusError, UserActionSender};
use common::executor::FutureSpawner;
use common::log::{debug, info, warn};
use futures::channel::oneshot;
use futures::future::{select, Either};
use mm2_err_handle::prelude::*;
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
    pub fn spawn_rpc_task<F>(this: &RpcTaskManagerShared<Task>, spawner: &F, mut task: Task) -> RpcTaskResult<TaskId>
    where
        F: FutureSpawner,
    {
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
                    info!("RPC task '{}' has been aborted", task_id);
                    task.cancel().await;
                },
            }
        };
        spawner.spawn(fut);
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
            TaskStatusExt::Ok(result) => RpcTaskStatus::Ok(result.clone()),
            TaskStatusExt::Error(error) => RpcTaskStatus::Error(error.clone()),
        };
        if rpc_status.is_ready() && forget_if_ready {
            entry.remove();
        }
        Some(rpc_status)
    }

    pub fn new_shared() -> RpcTaskManagerShared<Task> { Arc::new(Mutex::new(Self::default())) }

    pub fn contains(&self, task_id: TaskId) -> bool { self.tasks.contains_key(&task_id) }

    /// Cancel task if it's in progress.
    pub fn cancel_task(&mut self, task_id: TaskId) -> RpcTaskResult<()> {
        let task_status = match self.tasks.entry(task_id) {
            Entry::Occupied(task_status) => task_status,
            Entry::Vacant(_) => return MmError::err(RpcTaskError::NoSuchTask(task_id)),
        };

        if task_status.get().is_ready() {
            return MmError::err(RpcTaskError::UnexpectedTaskStatus {
                task_id,
                actual: TaskStatusError::Finished,
                expected: TaskStatusError::InProgress,
            });
        }

        // The task will be aborted when the `TaskAbortHandle` is dropped.
        task_status.remove();
        Ok(())
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
            TaskStatus::Ok(result) => self.on_task_finished(task_id, Ok(result)),
            TaskStatus::Error(error) => self.on_task_finished(task_id, Err(error)),
            TaskStatus::InProgress(in_progress) => self.update_in_progress_status(task_id, in_progress),
            TaskStatus::UserActionRequired {
                awaiting_status,
                user_action_tx,
            } => self.set_task_is_waiting_for_user_action(task_id, awaiting_status, user_action_tx),
        }
    }

    fn on_task_finished(
        &mut self,
        task_id: TaskId,
        task_result: MmResult<Task::Item, Task::Error>,
    ) -> RpcTaskResult<()> {
        let task_status = match task_result {
            Ok(result) => TaskStatusExt::Ok(result),
            Err(error) => TaskStatusExt::Error(error),
        };
        if self.tasks.insert(task_id, task_status).is_none() {
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
            Some(ready @ TaskStatusExt::Ok(_) | ready @ TaskStatusExt::Error(_)) => {
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
                let actual_status = unexpected.task_status_err();

                // Return the status to the tasks container.
                self.tasks.insert(task_id, unexpected);

                let error = RpcTaskError::UnexpectedTaskStatus {
                    task_id,
                    actual: actual_status,
                    expected: TaskStatusError::InProgress,
                };
                MmError::err(error)
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
                let actual_status = unexpected.task_status_err();

                // Return the unexpected status to the tasks container.
                self.tasks.insert(task_id, unexpected);

                let error = RpcTaskError::UnexpectedTaskStatus {
                    task_id,
                    actual: actual_status,
                    expected: TaskStatusError::AwaitingUserAction,
                };
                MmError::err(error)
            },
            None => MmError::err(RpcTaskError::NoSuchTask(task_id)),
        }
    }
}

/// `TaskStatus` extended with `TaskAbortHandle`.
/// This is stored in the [`RpcTaskManager::tasks`] container.
enum TaskStatusExt<Task: RpcTaskTypes> {
    Ok(Task::Item),
    Error(MmError<Task::Error>),
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
}

impl<Task: RpcTaskTypes> TaskStatusExt<Task> {
    fn is_ready(&self) -> bool { matches!(self, TaskStatusExt::Ok(_) | TaskStatusExt::Error(_)) }

    fn task_status_err(&self) -> TaskStatusError {
        match self {
            TaskStatusExt::Ok(_) | TaskStatusExt::Error(_) => TaskStatusError::Finished,
            TaskStatusExt::InProgress { .. } => TaskStatusError::InProgress,
            TaskStatusExt::Awaiting { .. } => TaskStatusError::AwaitingUserAction,
        }
    }
}
