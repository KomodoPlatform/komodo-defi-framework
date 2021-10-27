use async_trait::async_trait;
use std::collections::{HashMap, HashSet};

pub type Events<T> = Vec<T>;
pub type Listeners<EventType> = Vec<Box<dyn EventListener<Event = EventType> + Send + Sync>>;
pub type DispatchTable = HashMap<usize, Vec<usize>>;

pub trait EventUniqueIdTraits {
    fn get_event_unique_id(&self) -> usize;
}

#[async_trait]
pub trait EventListener {
    type Event;
    async fn process_event_async(&self, event: Self::Event);
    fn get_desired_events(&self) -> Events<Self::Event>;
}

#[derive(Default)]
pub struct Dispatcher<EventType> {
    listeners: Listeners<EventType>,
    listeners_id: HashSet<String>,
    dispatch_table: DispatchTable,
}

impl<EventType> Dispatcher<EventType>
where
    EventType: Clone + EventUniqueIdTraits,
{
    pub fn add_listener(
        &mut self,
        listener_id: &str,
        listen: impl EventListener<Event = EventType> + 'static + Send + Sync,
    ) {
        if self.listeners_id.contains(listener_id) {
            return;
        }
        self.listeners_id.insert(listener_id.to_string());
        let id = self.listeners.len();
        let events = listen.get_desired_events();
        self.listeners.push(Box::new(listen));
        for ev in events {
            self.dispatch_table
                .entry(ev.get_event_unique_id())
                .or_default()
                .push(id);
        }
    }

    pub async fn dispatch_async(&self, ev: EventType) {
        if let Some(ref interested) = self.dispatch_table.get(&ev.get_event_unique_id()) {
            for id in interested.iter().copied() {
                self.listeners[id].process_event_async(ev.clone()).await;
            }
        }
    }

    #[cfg(test)]
    pub fn nb_listeners(&self) -> usize { self.listeners.len() }
}

#[cfg(test)]
mod event_dispatcher_tests {
    use crate::block_on;
    use crate::event_dispatcher::{Dispatcher, EventListener, EventUniqueIdTraits, Events};
    use async_trait::async_trait;
    use std::ops::Deref;
    use std::sync::Arc;
    use uuid::Uuid;

    #[derive(Clone)]
    enum AppEvents {
        EventSwapStatusChanged { uuid: Uuid, status: String },
    }

    impl Default for AppEvents {
        // Just to satisfy the dispatcher
        fn default() -> Self {
            AppEvents::EventSwapStatusChanged {
                uuid: Default::default(),
                status: "".to_string(),
            }
        }
    }

    impl EventUniqueIdTraits for AppEvents {
        fn get_event_unique_id(&self) -> usize {
            match self {
                AppEvents::EventSwapStatusChanged { .. } => 0,
            }
        }
    }

    #[derive(Default)]
    struct ListenerSwapStatusChanged;

    #[derive(Clone)]
    struct ListenerSwapStatusChangedArc(Arc<ListenerSwapStatusChanged>);

    impl Deref for ListenerSwapStatusChangedArc {
        type Target = ListenerSwapStatusChanged;
        fn deref(&self) -> &ListenerSwapStatusChanged { &*self.0 }
    }

    #[async_trait]
    impl EventListener for ListenerSwapStatusChangedArc {
        type Event = AppEvents;

        async fn process_event_async(&self, event: Self::Event) {
            match event {
                AppEvents::EventSwapStatusChanged { uuid, status } => {
                    assert_eq!(uuid.to_string(), "00000000-0000-0000-0000-000000000000");
                    assert_eq!(status, "Started");
                },
            }
        }

        fn get_desired_events(&self) -> Events<AppEvents> {
            vec![AppEvents::EventSwapStatusChanged {
                uuid: Default::default(),
                status: "".to_string(),
            }]
        }
    }

    #[test]
    fn test_dispatcher() {
        let mut dispatcher: Dispatcher<AppEvents> = Default::default();
        let listener = ListenerSwapStatusChanged::default();
        let res = ListenerSwapStatusChangedArc(Arc::new(listener));
        dispatcher.add_listener("listener_swap_status_changed", res.clone());
        assert_eq!(dispatcher.nb_listeners(), 1);

        block_on(dispatcher.dispatch_async(AppEvents::EventSwapStatusChanged {
            uuid: Default::default(),
            status: "Started".to_string(),
        }));

        dispatcher.add_listener("listener_swap_status_changed", res);
        assert_eq!(dispatcher.nb_listeners(), 1);
    }
}
