//! The state-machine pattern implementation with the compile-time validation of the states transitions.
//!
//! See the usage examples in the `tests` module.

use crate::NotSame;
use async_trait::async_trait;

pub mod prelude {
    pub use super::{LastState, State, StateExt, StateResult, TransitionFrom};
}

pub trait TransitionFrom<Prev> {}

#[async_trait]
pub trait StateMachineTrait: Send + Sized + 'static {
    type Result: Send;

    async fn run(&mut self, mut state: Box<dyn State<StateMachine = Self>>) -> Self::Result {
        loop {
            let result = state.on_changed(self).await;
            match result {
                StateResult::ChangeState(ChangeGuard { next }) => {
                    state = next;
                },
                StateResult::Finish(ResultGuard { result }) => return result,
            };
        }
    }
}

/// Prevent implementing [`TransitionFrom<T>`] for `Next` If `T` implements `LastState` already.
impl<T, Next> !TransitionFrom<T> for Next
where
    T: LastState,
    // this bound is required to prevent conflicting implementation with `impl<T> !TransitionFrom<T> for T`.
    (T, Next): NotSame,
{
}

/// Prevent implementing [`TransitionFrom<T>`] for itself.
impl<T> !TransitionFrom<T> for T {}

#[async_trait]
pub trait State: Send + Sync + 'static {
    type StateMachine: StateMachineTrait;
    /// An action is called on entering this state.
    /// To change the state to another one in the end of processing, use [`StateExt::change_state`].
    /// For example:
    /// ```rust
    /// return Self::change_state(next_state);
    /// ```
    async fn on_changed(self: Box<Self>, ctx: &mut Self::StateMachine) -> StateResult<Self::StateMachine>;
}

pub trait StateExt {
    /// Change the state to the `next_state`.
    /// This function performs the compile-time validation whether this state can transition to the `Next` state,
    /// i.e checks if `Next` implements [`Transition::from(ThisState)`].
    fn change_state<Next>(next_state: Next) -> StateResult<Next::StateMachine>
    where
        Self: Sized,
        Next: State + TransitionFrom<Self>,
    {
        StateResult::ChangeState(ChangeGuard::next(next_state))
    }
}

impl<T: State> StateExt for T {}

#[async_trait]
pub trait LastState: Send + Sync + 'static {
    type StateMachine: StateMachineTrait;

    async fn on_changed(
        self: Box<Self>,
        ctx: &mut Self::StateMachine,
    ) -> <Self::StateMachine as StateMachineTrait>::Result;
}

#[async_trait]
impl<T: LastState> State for T {
    type StateMachine = T::StateMachine;

    /// The last state always returns the result of the state machine calculations.
    async fn on_changed(self: Box<Self>, ctx: &mut T::StateMachine) -> StateResult<T::StateMachine> {
        let result = LastState::on_changed(self, ctx).await;
        StateResult::Finish(ResultGuard::new(result))
    }
}

pub enum StateResult<Machine: StateMachineTrait> {
    ChangeState(ChangeGuard<Machine>),
    Finish(ResultGuard<Machine::Result>),
}

/* vvv The access guards that prevents the user using this pattern from entering an invalid state vvv */

/// An instance of `ChangeGuard` can be initialized within `state_machine` module only.
pub struct ChangeGuard<Machine: StateMachineTrait> {
    /// The private field.
    next: Box<dyn State<StateMachine = Machine>>,
}

impl<Machine: StateMachineTrait + 'static> ChangeGuard<Machine> {
    /// The private constructor.
    fn next<Next: State<StateMachine = Machine>>(next_state: Next) -> Self {
        ChangeGuard {
            next: Box::new(next_state),
        }
    }
}

/// An instance of `ResultGuard` can be initialized within `state_machine` module only.
pub struct ResultGuard<T> {
    /// The private field.
    result: T,
}

impl<T> ResultGuard<T> {
    /// The private constructor.
    fn new(result: T) -> Self { ResultGuard { result } }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block_on;
    use crate::executor::spawn;
    use futures::channel::mpsc;
    use futures::{SinkExt, StreamExt};
    use std::collections::HashMap;

    type UserId = usize;
    type Login = String;
    type Password = String;

    #[derive(Debug, PartialEq)]
    enum ErrorType {
        UnexpectedCredentialsFormat,
        UnknownUser,
    }

    struct AuthStateMachine {
        users: HashMap<(Login, Password), UserId>,
    }

    type AuthResult = Result<UserId, ErrorType>;

    impl StateMachineTrait for AuthStateMachine {
        type Result = AuthResult;
    }

    struct ReadingState {
        rx: mpsc::Receiver<char>,
    }
    struct ParsingState {
        line: String,
    }
    struct AuthenticationState {
        login: Login,
        password: Password,
    }
    struct SuccessfulState {
        user_id: UserId,
    }
    struct ErrorState {
        error: ErrorType,
    }

    impl TransitionFrom<ReadingState> for ParsingState {}
    impl TransitionFrom<ParsingState> for AuthenticationState {}
    impl TransitionFrom<ParsingState> for ErrorState {}
    impl TransitionFrom<AuthenticationState> for SuccessfulState {}
    impl TransitionFrom<AuthenticationState> for ErrorState {}

    #[async_trait]
    impl LastState for SuccessfulState {
        type StateMachine = AuthStateMachine;

        async fn on_changed(self: Box<Self>, _ctx: &mut AuthStateMachine) -> AuthResult { Ok(self.user_id) }
    }

    #[async_trait]
    impl LastState for ErrorState {
        type StateMachine = AuthStateMachine;

        async fn on_changed(self: Box<Self>, _ctx: &mut AuthStateMachine) -> AuthResult { Err(self.error) }
    }

    #[async_trait]
    impl State for ReadingState {
        type StateMachine = AuthStateMachine;

        async fn on_changed(mut self: Box<Self>, _ctx: &mut AuthStateMachine) -> StateResult<AuthStateMachine> {
            let mut line = String::with_capacity(80);
            while let Some(ch) = self.rx.next().await {
                line.push(ch);
            }
            let next_state = ParsingState { line };
            Self::change_state(next_state)
        }
    }

    #[async_trait]
    impl State for ParsingState {
        type StateMachine = AuthStateMachine;

        async fn on_changed(self: Box<Self>, _ctx: &mut AuthStateMachine) -> StateResult<AuthStateMachine> {
            // parse the line into two chunks: (login, password)
            let chunks: Vec<_> = self.line.split(' ').collect();
            if chunks.len() == 2 {
                let next_state = AuthenticationState {
                    login: chunks[0].to_owned(),
                    password: chunks[1].to_owned(),
                };
                return Self::change_state(next_state);
            }

            let error_state = ErrorState {
                error: ErrorType::UnexpectedCredentialsFormat,
            };
            Self::change_state(error_state)
        }
    }

    #[async_trait]
    impl State for AuthenticationState {
        type StateMachine = AuthStateMachine;

        async fn on_changed(self: Box<Self>, ctx: &mut AuthStateMachine) -> StateResult<AuthStateMachine> {
            let credentials = (self.login, self.password);
            match ctx.users.get(&credentials) {
                Some(user_id) => Self::change_state(SuccessfulState { user_id: *user_id }),
                None => Self::change_state(ErrorState {
                    error: ErrorType::UnknownUser,
                }),
            }
        }
    }

    fn run_auth_machine(credentials: &'static str) -> Result<UserId, ErrorType> {
        let (mut tx, rx) = mpsc::channel(80);

        let mut users = HashMap::new();
        users.insert(("user1".to_owned(), "password1".to_owned()), 1);
        users.insert(("user2".to_owned(), "password2".to_owned()), 2);
        users.insert(("user3".to_owned(), "password3".to_owned()), 3);

        let fut = async move {
            for ch in credentials.chars() {
                tx.send(ch).await.expect("!tx.try_send()");
            }
        };
        spawn(fut);

        let fut = async move {
            let initial_state: ReadingState = ReadingState { rx };
            let mut state_machine = AuthStateMachine { users };
            state_machine.run(Box::new(initial_state)).await
        };
        block_on(fut)
    }

    #[test]
    fn test_state_machine() {
        let actual = run_auth_machine("user3 password3");
        assert_eq!(actual, Ok(3));
    }

    #[test]
    fn test_state_machine_error() {
        const INVALID_CREDENTIALS: &str = "invalid_format";
        const UNKNOWN_USER: &str = "user4 password4";

        let actual = run_auth_machine(INVALID_CREDENTIALS);
        assert_eq!(actual, Err(ErrorType::UnexpectedCredentialsFormat));

        let actual = run_auth_machine(UNKNOWN_USER);
        assert_eq!(actual, Err(ErrorType::UnknownUser));
    }
}
