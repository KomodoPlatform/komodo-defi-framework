use crate::prelude::*;
use crate::state_machine::ChangeGuard;
use async_trait::async_trait;

#[async_trait]
pub trait OnNewState<S> {
    async fn on_new_state(&self, state: &S);
}

#[async_trait]
pub trait EventStorage {
    type Event: Send;

    async fn store_events(&self, events: Vec<Self::Event>);
}

#[async_trait]
pub trait StorableStateMachine: StateMachineTrait {
    type Storage: EventStorage + Sync;

    fn storage(&self) -> &Self::Storage;

    async fn store_events(&self, events: Vec<<Self::Storage as EventStorage>::Event>) {
        self.storage().store_events(events).await
    }
}

pub trait StorableState {
    type StateMachine: StorableStateMachine;

    fn get_events(&self) -> Vec<<<Self::StateMachine as StorableStateMachine>::Storage as EventStorage>::Event>;
}

#[async_trait]
impl<T: StorableStateMachine + Sync, S: StorableState<StateMachine = T> + Sync> OnNewState<S> for T {
    async fn on_new_state(&self, state: &S) {
        let events = state.get_events();
        self.store_events(events).await;
    }
}

#[async_trait]
pub trait ChangeStateOnNewExt {
    /// Change the state to the `next_state`.
    /// This function performs the compile-time validation whether this state can transition to the `Next` state,
    /// i.e checks if `Next` implements [`Transition::from(ThisState)`].
    async fn change_state<Next>(next_state: Next, machine: &Next::StateMachine) -> StateResult<Next::StateMachine>
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
    use common::block_on;

    struct StorageTest {}

    struct StorableStateMachineTest {
        storage: StorageTest,
    }

    impl StateMachineTrait for StorableStateMachineTest {
        type Result = ();
    }

    enum TestEvent {
        ForStateTwo,
    }

    #[async_trait]
    impl EventStorage for StorageTest {
        type Event = TestEvent;

        async fn store_events(&self, _events: Vec<Self::Event>) { todo!() }
    }

    impl StorableStateMachine for StorableStateMachineTest {
        type Storage = StorageTest;

        fn storage(&self) -> &Self::Storage { &self.storage }
    }

    struct StateOne {}

    impl StorableState for StateOne {
        type StateMachine = StorableStateMachineTest;

        fn get_events(&self) -> Vec<TestEvent> { vec![] }
    }

    struct StateTwo {}

    impl StorableState for StateTwo {
        type StateMachine = StorableStateMachineTest;

        fn get_events(&self) -> Vec<TestEvent> { vec![TestEvent::ForStateTwo] }
    }

    impl TransitionFrom<StateOne> for StateTwo {}

    #[async_trait]
    impl LastState for StateTwo {
        type StateMachine = StorableStateMachineTest;

        async fn on_changed(self: Box<Self>, _ctx: &mut Self::StateMachine) -> () {}
    }

    #[async_trait]
    impl State for StateOne {
        type StateMachine = StorableStateMachineTest;

        async fn on_changed(self: Box<Self>, ctx: &mut Self::StateMachine) -> StateResult<Self::StateMachine> {
            Self::change_state(StateTwo {}, ctx).await
        }
    }

    #[test]
    fn run_storable_state_machine() {
        let mut machine = StorableStateMachineTest {
            storage: StorageTest {},
        };
        block_on(machine.run(Box::new(StateOne {})));
    }
}
