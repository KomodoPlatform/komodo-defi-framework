use async_trait::async_trait;

use crate::prelude::*;
use crate::state_machine::{ChangeGuard, ErrorGuard};

/// A trait representing the initial state of a state machine.
pub trait InitialState {
    /// The type of state machine associated with this initial state.
    type StateMachine: StorableStateMachine;
}

/// A trait for handling new states in a state machine.
#[async_trait]
pub trait OnNewState<S>: StateMachineTrait {
    /// Handles a new state.
    ///
    /// # Parameters
    ///
    /// - `state`: A reference to the new state to be handled.
    ///
    /// # Returns
    ///
    /// A `Result` indicating success (`Ok(())`) or an error (`Err(<Self as StateMachineTrait>::Error)`).
    async fn on_new_state(&mut self, state: &S) -> Result<(), <Self as StateMachineTrait>::Error>;
}

/// A trait for the storage of state machine events.
#[async_trait]
pub trait StateMachineStorage: Send + Sync {
    /// The type representing a unique identifier for a state machine.
    type MachineId: Send;
    /// The type representing an event that can be stored.
    type Event: Send;
    /// The type representing an error that can occur during storage operations.
    type Error: Send;

    /// Stores an event for a given state machine.
    ///
    /// # Parameters
    ///
    /// - `id`: The unique identifier of the state machine.
    /// - `event`: The event to be stored.
    ///
    /// # Returns
    ///
    /// A `Result` indicating success (`Ok(())`) or an error (`Err(Self::Error)`).
    async fn store_event(&mut self, id: Self::MachineId, event: Self::Event) -> Result<(), Self::Error>;

    /// Retrieves a list of unfinished state machines.
    ///
    /// # Returns
    ///
    /// A `Result` containing a vector of machine IDs or an error (`Err(Self::Error)`).
    async fn get_unfinished(&self) -> Result<Vec<Self::MachineId>, Self::Error>;

    /// Marks a state machine as finished.
    ///
    /// # Parameters
    ///
    /// - `id`: The unique identifier of the state machine to be marked as finished.
    ///
    /// # Returns
    ///
    /// A `Result` indicating success (`Ok(())`) or an error (`Err(Self::Error)`).
    async fn mark_finished(&mut self, id: Self::MachineId) -> Result<(), Self::Error>;
}

/// A struct representing a restored state machine.
#[allow(dead_code)]
pub struct RestoredMachine<M> {
    machine: M,
    current_state: Box<dyn State<StateMachine = M>>,
}

/// A trait for storable state machines.
#[async_trait]
pub trait StorableStateMachine: Send + Sized + 'static {
    /// The type of storage for the state machine.
    type Storage: StateMachineStorage;
    /// The result type of the state machine.
    type Result: Send;

    /// Gets a mutable reference to the storage for the state machine.
    fn storage(&mut self) -> &mut Self::Storage;

    /// Gets the unique identifier of the state machine.
    fn id(&self) -> <Self::Storage as StateMachineStorage>::MachineId;

    /// Restores a state machine from storage.
    ///
    /// # Parameters
    ///
    /// - `id`: The unique identifier of the state machine to be restored.
    /// - `storage`: The storage containing the state machine's data.
    ///
    /// # Returns
    ///
    /// A `Result` containing a `RestoredMachine` or an error.
    fn restore_from_storage(
        id: <Self::Storage as StateMachineStorage>::MachineId,
        storage: Self::Storage,
    ) -> Result<RestoredMachine<Self>, <Self::Storage as StateMachineStorage>::Error>;

    /// Stores an event for the state machine.
    ///
    /// # Parameters
    ///
    /// - `event`: The event to be stored.
    ///
    /// # Returns
    ///
    /// A `Result` indicating success (`Ok(())`) or an error (`Err(Self::Error)`).
    async fn store_event(
        &mut self,
        event: <Self::Storage as StateMachineStorage>::Event,
    ) -> Result<(), <Self::Storage as StateMachineStorage>::Error> {
        let id = self.id();
        self.storage().store_event(id, event).await
    }

    /// Marks the state machine as finished.
    ///
    /// # Returns
    ///
    /// A `Result` indicating success (`Ok(())`) or an error (`Err(Self::Error)`).
    async fn mark_finished(&mut self) -> Result<(), <Self::Storage as StateMachineStorage>::Error> {
        let id = self.id();
        self.storage().mark_finished(id).await
    }
}

// Ensure that StandardStateMachine won't be occasionally implemented for StorableStateMachine.
// Users of StorableStateMachine must be prevented from using ChangeStateExt::change_state
// because it doesn't call machine.on_new_state.
impl<T: StorableStateMachine> !StandardStateMachine for T {}
// Prevent implementing both StorableState and InitialState at the same time
impl<T: StorableState> !InitialState for T {}

#[async_trait]
impl<T: StorableStateMachine> StateMachineTrait for T {
    type Result = T::Result;
    type Error = <T::Storage as StateMachineStorage>::Error;

    async fn on_finished(&mut self) -> Result<(), <T::Storage as StateMachineStorage>::Error> {
        self.mark_finished().await
    }
}

/// A trait for storable states.
pub trait StorableState {
    /// The type of state machine associated with this state.
    type StateMachine: StorableStateMachine;

    /// Gets the event associated with this state.
    fn get_event(&self) -> <<Self::StateMachine as StorableStateMachine>::Storage as StateMachineStorage>::Event;
}

/// Implementation of `OnNewState` for storable state machines and their related states.
#[async_trait]
impl<T: StorableStateMachine + Sync, S: StorableState<StateMachine = T> + Sync> OnNewState<S> for T {
    /// Handles a new state.
    ///
    /// # Parameters
    ///
    /// - `state`: A reference to the new state to be handled.
    ///
    /// # Returns
    ///
    /// A `Result` indicating success (`Ok(())`) or an error (`Err(Self::Error)`).
    async fn on_new_state(&mut self, state: &S) -> Result<(), <T::Storage as StateMachineStorage>::Error> {
        let event = state.get_event();
        self.store_event(event).await
    }
}

/// An asynchronous function for changing the state of a storable state machine.
///
/// # Parameters
///
/// - `next_state`: The next state to transition to.
/// - `machine`: A mutable reference to the state machine.
///
/// # Returns
///
/// A `StateResult` indicating success or an error.
///
/// # Generic Parameters
///
/// - `Next`: The type of the next state.
async fn change_state_impl<Next>(next_state: Next, machine: &mut Next::StateMachine) -> StateResult<Next::StateMachine>
where
    Next: State + ChangeStateOnNewExt,
    Next::StateMachine: OnNewState<Next> + Sync,
{
    if let Err(e) = machine.on_new_state(&next_state).await {
        return StateResult::Error(ErrorGuard::new(e));
    }
    StateResult::ChangeState(ChangeGuard::next(next_state))
}

/// A trait for state transition functionality.
#[async_trait]
pub trait ChangeStateOnNewExt {
    /// Change the state to the `next_state`.
    ///
    /// # Parameters
    ///
    /// - `next_state`: The next state to transition to.
    /// - `machine`: A mutable reference to the state machine.
    ///
    /// # Returns
    ///
    /// A `StateResult` indicating success or an error.
    ///
    /// # Generic Parameters
    ///
    /// - `Next`: The type of the next state.
    async fn change_state<Next>(next_state: Next, machine: &mut Next::StateMachine) -> StateResult<Next::StateMachine>
    where
        Self: Sized,
        Next: State + TransitionFrom<Self> + ChangeStateOnNewExt,
        Next::StateMachine: OnNewState<Next> + Sync,
    {
        change_state_impl(next_state, machine).await
    }
}

impl<M: StorableStateMachine, T: StorableState<StateMachine = M>> ChangeStateOnNewExt for T {}

/// A trait for initial state change functionality.
#[async_trait]
pub trait ChangeInitialStateExt: InitialState {
    /// Change the state to the `next_state`.
    ///
    /// # Parameters
    ///
    /// - `next_state`: The next state to transition to.
    /// - `machine`: A mutable reference to the state machine.
    ///
    /// # Returns
    ///
    /// A `StateResult` indicating success or an error.
    ///
    /// # Generic Parameters
    ///
    /// - `Next`: The type of the next state.
    async fn change_state<Next>(next_state: Next, machine: &mut Next::StateMachine) -> StateResult<Next::StateMachine>
    where
        Self: Sized,
        Next: State + TransitionFrom<Self> + ChangeStateOnNewExt,
        Next::StateMachine: OnNewState<Next> + Sync,
    {
        change_state_impl(next_state, machine).await
    }
}

impl<M: StorableStateMachine, T: InitialState<StateMachine = M>> ChangeInitialStateExt for T {}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::convert::Infallible;

    use common::block_on;

    use super::*;

    struct StorageTest {
        events_unfinished: HashMap<usize, Vec<TestEvent>>,
        events_finished: HashMap<usize, Vec<TestEvent>>,
    }

    impl StorageTest {
        fn empty() -> Self {
            StorageTest {
                events_unfinished: HashMap::new(),
                events_finished: HashMap::new(),
            }
        }
    }

    struct StorableStateMachineTest {
        id: usize,
        storage: StorageTest,
    }

    #[derive(Debug, Eq, PartialEq)]
    enum TestEvent {
        ForState2,
        ForState3,
        ForState4,
    }

    #[async_trait]
    impl StateMachineStorage for StorageTest {
        type MachineId = usize;
        type Event = TestEvent;
        type Error = Infallible;

        async fn store_event(&mut self, machine_id: usize, events: Self::Event) -> Result<(), Self::Error> {
            self.events_unfinished
                .entry(machine_id)
                .or_insert_with(Vec::new)
                .push(events);
            Ok(())
        }

        async fn get_unfinished(&self) -> Result<Vec<Self::MachineId>, Self::Error> {
            Ok(self.events_unfinished.keys().copied().collect())
        }

        async fn mark_finished(&mut self, id: Self::MachineId) -> Result<(), Self::Error> {
            let events = self.events_unfinished.remove(&id).unwrap();
            self.events_finished.insert(id, events);
            Ok(())
        }
    }

    impl StorableStateMachine for StorableStateMachineTest {
        type Storage = StorageTest;
        type Result = ();

        fn storage(&mut self) -> &mut Self::Storage { &mut self.storage }

        fn id(&self) -> <Self::Storage as StateMachineStorage>::MachineId { self.id }

        fn restore_from_storage(
            id: <Self::Storage as StateMachineStorage>::MachineId,
            storage: Self::Storage,
        ) -> Result<RestoredMachine<Self>, <Self::Storage as StateMachineStorage>::Error> {
            let events = storage.events_unfinished.get(&id).unwrap();
            let current_state: Box<dyn State<StateMachine = Self>> = match events.last() {
                None => Box::new(State1 {}),
                Some(TestEvent::ForState2) => Box::new(State2 {}),
                _ => unimplemented!(),
            };
            let machine = StorableStateMachineTest { id, storage };
            Ok(RestoredMachine { machine, current_state })
        }
    }

    struct State1 {}

    impl InitialState for State1 {
        type StateMachine = StorableStateMachineTest;
    }

    struct State2 {}

    impl StorableState for State2 {
        type StateMachine = StorableStateMachineTest;

        fn get_event(&self) -> TestEvent { TestEvent::ForState2 }
    }

    impl TransitionFrom<State1> for State2 {}

    struct State3 {}

    impl StorableState for State3 {
        type StateMachine = StorableStateMachineTest;

        fn get_event(&self) -> TestEvent { TestEvent::ForState3 }
    }

    impl TransitionFrom<State2> for State3 {}

    struct State4 {}

    impl StorableState for State4 {
        type StateMachine = StorableStateMachineTest;

        fn get_event(&self) -> TestEvent { TestEvent::ForState4 }
    }

    impl TransitionFrom<State3> for State4 {}

    #[async_trait]
    impl LastState for State4 {
        type StateMachine = StorableStateMachineTest;

        async fn on_changed(self: Box<Self>, _ctx: &mut Self::StateMachine) -> () {}
    }

    #[async_trait]
    impl State for State1 {
        type StateMachine = StorableStateMachineTest;

        async fn on_changed(self: Box<Self>, ctx: &mut Self::StateMachine) -> StateResult<Self::StateMachine> {
            Self::change_state(State2 {}, ctx).await
        }
    }

    #[async_trait]
    impl State for State2 {
        type StateMachine = StorableStateMachineTest;

        async fn on_changed(self: Box<Self>, ctx: &mut Self::StateMachine) -> StateResult<Self::StateMachine> {
            Self::change_state(State3 {}, ctx).await
        }
    }

    #[async_trait]
    impl State for State3 {
        type StateMachine = StorableStateMachineTest;

        async fn on_changed(self: Box<Self>, ctx: &mut Self::StateMachine) -> StateResult<Self::StateMachine> {
            Self::change_state(State4 {}, ctx).await
        }
    }

    #[test]
    fn run_storable_state_machine() {
        let mut machine = StorableStateMachineTest {
            id: 1,
            storage: StorageTest::empty(),
        };
        block_on(machine.run(Box::new(State1 {}))).unwrap();

        let expected_events = HashMap::from_iter([(1, vec![
            TestEvent::ForState2,
            TestEvent::ForState3,
            TestEvent::ForState4,
        ])]);
        assert_eq!(expected_events, machine.storage.events_finished);
    }

    #[test]
    fn restore_state_machine() {
        let mut storage = StorageTest::empty();
        let id = 1;
        storage.events_unfinished.insert(1, vec![TestEvent::ForState2]);
        let RestoredMachine {
            mut machine,
            current_state,
        } = StorableStateMachineTest::restore_from_storage(id, storage).unwrap();

        block_on(machine.run(current_state)).unwrap();

        let expected_events = HashMap::from_iter([(1, vec![
            TestEvent::ForState2,
            TestEvent::ForState3,
            TestEvent::ForState4,
        ])]);
        assert_eq!(expected_events, machine.storage.events_finished);
    }
}
