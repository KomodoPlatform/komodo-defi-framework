pub use crate::state_machine::{ChangeStateExt, LastState, State, StateMachineTrait, StateResult};

pub trait TransitionFrom<Prev> {}
pub trait StandardStateMachine {}
