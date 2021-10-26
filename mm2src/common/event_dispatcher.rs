use std::collections::HashMap;

pub type Events<T> = Vec<T>;
pub type Listeners<EventType> = Vec<Box<dyn EventListener<Event = EventType>>>;
pub type DispatchTable = HashMap<usize, Vec<usize>>;

pub trait EventUniqueIdTraits {
    fn get_event_unique_id(&self) -> usize;
}

pub trait EventListener {
    type Event;
    fn process_event(&self, event: Self::Event);
    fn get_desired_events(&self) -> Events<Self::Event>;
}

#[derive(Default)]
pub struct Dispatcher<EventType> {
    listeners: Listeners<EventType>,
    dispatch_table: DispatchTable,
}

impl<EventType> Dispatcher<EventType>
where
    EventType: Clone + EventUniqueIdTraits,
{
    pub fn add_listener(&mut self, listen: impl EventListener<Event = EventType> + 'static + Send + Sync) {
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

    pub fn dispatch(&self, ev: EventType) {
        if let Some(ref interested) = self.dispatch_table.get(&ev.get_event_unique_id()) {
            for id in interested.iter().copied() {
                self.listeners[id].process_event(ev.clone());
            }
        }
    }
}

#[cfg(test)]
mod event_dispatcher_tests {
    use crate::event_dispatcher::{Dispatcher, EventListener, EventUniqueIdTraits, Events};
    use std::ops::Deref;
    use std::sync::Arc;
    use uuid::Uuid;

    #[derive(Clone)]
    pub enum AppEvents {
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
    pub struct ListenerSwapStatusChanged;

    #[derive(Clone)]
    pub struct ListenerSwapStatusChangedArc(Arc<ListenerSwapStatusChanged>);

    impl Deref for ListenerSwapStatusChangedArc {
        type Target = ListenerSwapStatusChanged;
        fn deref(&self) -> &ListenerSwapStatusChanged { &*self.0 }
    }

    impl EventListener for ListenerSwapStatusChanged {
        type Event = AppEvents;
        fn process_event(&self, ev: AppEvents) {
            match ev {
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

    impl<El: EventListener<Event = AppEvents>, Ptr: Deref<Target = El>> EventListener for Ptr {
        type Event = AppEvents;
        fn process_event(&self, ev: Self::Event) { self.deref().process_event(ev); }
        fn get_desired_events(&self) -> Events<Self::Event> { self.deref().get_desired_events() }
    }

    #[test]
    fn test_dispatcher() {
        let mut dispatcher: Dispatcher<AppEvents> = Default::default();
        let listener = ListenerSwapStatusChanged::default();
        let res = ListenerSwapStatusChangedArc(Arc::new(listener));
        dispatcher.add_listener(res);
        dispatcher.dispatch(AppEvents::EventSwapStatusChanged {
            uuid: Default::default(),
            status: "Started".to_string(),
        });
    }
}
