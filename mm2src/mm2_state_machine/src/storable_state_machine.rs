use crate::prelude::*;
use crate::state_machine::ChangeGuard;
use async_trait::async_trait;

#[async_trait]
pub trait OnNewState<S> {
    async fn on_new_state(&mut self, state: &S);
}

#[async_trait]
pub trait StateMachineStorage: Send + Sync {
    type MachineId: Send;
    type Event: Send;

    async fn store_events(&mut self, id: Self::MachineId, events: Vec<Self::Event>);

    async fn get_unfinished(&self) -> Vec<Self::MachineId>;

    async fn mark_finished(&self, id: Self::MachineId);
}

#[async_trait]
pub trait StorableStateMachine: StateMachineTrait {
    type Storage: StateMachineStorage;

    fn storage(&mut self) -> &mut Self::Storage;

    fn id(&self) -> <Self::Storage as StateMachineStorage>::MachineId;

    fn restore_from_storage(
        id: <Self::Storage as StateMachineStorage>::MachineId,
        storage: Self::Storage,
    ) -> (Self, Box<dyn State<StateMachine = Self>>);

    async fn store_events(&mut self, events: Vec<<Self::Storage as StateMachineStorage>::Event>) {
        let id = self.id();
        self.storage().store_events(id, events).await
    }

    async fn mark_finished(&mut self) {
        let id = self.id();
        self.storage().mark_finished(id).await
    }
}

pub trait StorableState {
    type StateMachine: StorableStateMachine;

    fn get_events(&self) -> Vec<<<Self::StateMachine as StorableStateMachine>::Storage as StateMachineStorage>::Event>;
}

#[async_trait]
impl<T: StorableStateMachine + Sync, S: StorableState<StateMachine = T> + Sync> OnNewState<S> for T {
    async fn on_new_state(&mut self, state: &S) {
        let events = state.get_events();
        self.store_events(events).await;
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
        Next::StateMachine: OnNewState<Next> + Sync,
    {
        machine.on_new_state(&next_state).await;
        StateResult::ChangeState(ChangeGuard::next(next_state))
    }
}

// Users of StorableStateMachine must be prevented from using ChangeStateExt::change_state
// because it doesn't call machine.on_new_state
// This prevents ChangeStateExt to be implemented StorableStateMachine's states
impl<T: StorableStateMachine> !StandardStateMachine for T {}
impl<S: OnNewState<T>, T: State<StateMachine = S>> ChangeStateOnNewExt for T {}

mod tests {
    use super::*;
    use crate::storable_state_machine::tests::TestEvent::{ForState3First, ForState3Second};
    use common::block_on;
    use std::collections::HashMap;

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

    impl StateMachineTrait for StorableStateMachineTest {
        type Result = ();
    }

    #[derive(Debug, Eq, PartialEq)]
    enum TestEvent {
        ForState2,
        ForState3First,
        ForState3Second,
    }

    #[async_trait]
    impl StateMachineStorage for StorageTest {
        type MachineId = usize;
        type Event = TestEvent;

        async fn store_events(&mut self, machine_id: usize, events: Vec<Self::Event>) {
            self.events_unfinished
                .entry(machine_id)
                .or_insert_with(Vec::new)
                .extend(events)
        }

        async fn get_unfinished(&self) -> Vec<Self::MachineId> { vec![] }

        async fn mark_finished(&self, id: Self::MachineId) { todo!() }
    }

    impl StorableStateMachine for StorableStateMachineTest {
        type Storage = StorageTest;

        fn storage(&mut self) -> &mut Self::Storage { &mut self.storage }

        fn id(&self) -> <Self::Storage as StateMachineStorage>::MachineId { self.id }

        fn restore_from_storage(
            id: <Self::Storage as StateMachineStorage>::MachineId,
            storage: Self::Storage,
        ) -> (Self, Box<dyn State<StateMachine = Self>>) {
            let events = storage.events_unfinished.get(&id).unwrap().clone();
            match events.last() {
                Some(TestEvent::ForState2) => (StorableStateMachineTest { id, storage }, Box::new(State2 {})),
                _ => unimplemented!(),
            }
        }
    }

    struct State1 {}

    impl StorableState for State1 {
        type StateMachine = StorableStateMachineTest;

        fn get_events(&self) -> Vec<TestEvent> { vec![] }
    }

    struct State2 {}

    impl StorableState for State2 {
        type StateMachine = StorableStateMachineTest;

        fn get_events(&self) -> Vec<TestEvent> { vec![TestEvent::ForState2] }
    }

    impl TransitionFrom<State1> for State2 {}

    struct State3 {}

    impl StorableState for State3 {
        type StateMachine = StorableStateMachineTest;

        fn get_events(&self) -> Vec<TestEvent> { vec![TestEvent::ForState3First, ForState3Second] }
    }

    impl TransitionFrom<State2> for State3 {}

    #[async_trait]
    impl LastState for State3 {
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

    #[test]
    fn run_storable_state_machine() {
        let mut machine = StorableStateMachineTest {
            id: 1,
            storage: StorageTest::empty(),
        };
        block_on(machine.run(Box::new(State1 {})));

        let expected_events = HashMap::from_iter([(1, vec![TestEvent::ForState2, ForState3First, ForState3Second])]);
        assert_eq!(expected_events, machine.storage.events_finished);
    }

    #[test]
    fn restore_state_machine() {
        let mut storage = StorageTest::empty();
        let id = 1;
        storage.events_unfinished.insert(1, vec![TestEvent::ForState2]);
        let (mut machine, state) = StorableStateMachineTest::restore_from_storage(id, storage);

        block_on(machine.run(state));

        let expected_events = HashMap::from_iter([(1, vec![TestEvent::ForState2, ForState3First, ForState3Second])]);
        assert_eq!(expected_events, machine.storage.events_finished);
    }
}
