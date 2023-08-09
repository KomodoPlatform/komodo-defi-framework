use crate::prelude::*;
use crate::state_machine::{ChangeGuard, ErrorGuard};
use async_trait::async_trait;

#[async_trait]
pub trait OnNewState<S> {
    type Error;

    async fn on_new_state(&mut self, state: &S) -> Result<(), Self::Error>;
}

#[async_trait]
pub trait StateMachineStorage: Send + Sync {
    type MachineId: Send;
    type Event: Send;
    type Error: Send;

    async fn store_event(&mut self, id: Self::MachineId, event: Self::Event) -> Result<(), Self::Error>;

    async fn get_unfinished(&self) -> Result<Vec<Self::MachineId>, Self::Error>;

    async fn mark_finished(&mut self, id: Self::MachineId) -> Result<(), Self::Error>;
}

#[allow(dead_code)]
pub struct RestoredMachine<M> {
    machine: M,
    current_state: Box<dyn State<StateMachine = M>>,
}

#[async_trait]
pub trait StorableStateMachine: Send + Sized + 'static {
    type Storage: StateMachineStorage;
    type Result: Send;

    fn storage(&mut self) -> &mut Self::Storage;

    fn id(&self) -> <Self::Storage as StateMachineStorage>::MachineId;

    fn restore_from_storage(
        id: <Self::Storage as StateMachineStorage>::MachineId,
        storage: Self::Storage,
    ) -> Result<RestoredMachine<Self>, <Self::Storage as StateMachineStorage>::Error>;

    async fn store_event(
        &mut self,
        event: <Self::Storage as StateMachineStorage>::Event,
    ) -> Result<(), <Self::Storage as StateMachineStorage>::Error> {
        let id = self.id();
        self.storage().store_event(id, event).await
    }

    async fn mark_finished(&mut self) -> Result<(), <Self::Storage as StateMachineStorage>::Error> {
        let id = self.id();
        self.storage().mark_finished(id).await
    }
}

#[async_trait]
impl<T: StorableStateMachine> StateMachineTrait for T {
    type Result = T::Result;
    type Error = <T::Storage as StateMachineStorage>::Error;

    async fn on_finished(&mut self) -> Result<(), <T::Storage as StateMachineStorage>::Error> {
        self.mark_finished().await
    }
}

pub trait StorableState {
    type StateMachine: StorableStateMachine;

    fn get_event(&self) -> <<Self::StateMachine as StorableStateMachine>::Storage as StateMachineStorage>::Event;
}

#[async_trait]
impl<T: StorableStateMachine + Sync, S: StorableState<StateMachine = T> + Sync> OnNewState<S> for T {
    type Error = <T::Storage as StateMachineStorage>::Error;

    async fn on_new_state(&mut self, state: &S) -> Result<(), <T::Storage as StateMachineStorage>::Error> {
        let event = state.get_event();
        self.store_event(event).await
    }
}

#[async_trait]
pub trait ChangeStateOnNewExt {
    /// Change the state to the `next_state`.
    /// This function performs the compile-time validation whether this state can transition to the `Next` state,
    /// i.e checks if `Next` implements [`Transition::from(ThisState)`].
    async fn change_state<Next>(next_state: Next, machine: &mut Next::StateMachine) -> StateResult<Next::StateMachine>
    where
        Self: Sized,
        Next: State + TransitionFrom<Self> + ChangeStateOnNewExt,
        Next::StateMachine: OnNewState<Next, Error = <Next::StateMachine as StateMachineTrait>::Error> + Sync,
    {
        if let Err(e) = machine.on_new_state(&next_state).await {
            return StateResult::Error(ErrorGuard::new(e));
        }
        StateResult::ChangeState(ChangeGuard::next(next_state))
    }
}

// Users of StorableStateMachine must be prevented from using ChangeStateExt::change_state
// because it doesn't call machine.on_new_state
// This prevents ChangeStateExt to be implemented StorableStateMachine's states
impl<T: StorableStateMachine> !StandardStateMachine for T {}
impl<S: OnNewState<T>, T: State<StateMachine = S>> ChangeStateOnNewExt for T {}

#[cfg(test)]
mod tests {
    use super::*;
    use common::block_on;
    use std::collections::HashMap;
    use std::convert::Infallible;

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

    /*
    #[async_trait]
    impl OnNewState<State1> for StorableStateMachineTest {
        type Error = Infallible;

        async fn on_new_state(&mut self, _state: &State1) -> Result<(), Self::Error> { Ok(()) }
    }
     */

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
