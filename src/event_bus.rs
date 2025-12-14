//! Event bus for inter-handler communication.
//!
//! The EventBus manages channels that allow handlers to forward events to each other.
//! Each handler registers with the EventBus to receive a channel for incoming events,
//! and can send events to other handlers by name.

use crate::event::Event;
use log::{debug, warn};
use std::collections::HashMap;
use std::sync::RwLock;
use tokio::sync::mpsc;

/// Errors that can occur when using the EventBus
#[derive(Debug, Clone, PartialEq)]
pub enum EventBusError {
    /// The target handler is not registered
    HandlerNotFound { handler: String },
    /// Failed to send event (channel closed)
    SendFailed { handler: String },
    /// Handler is already registered
    AlreadyRegistered { handler: String },
}

impl std::fmt::Display for EventBusError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EventBusError::HandlerNotFound { handler } => {
                write!(f, "Handler '{}' not found in event bus", handler)
            }
            EventBusError::SendFailed { handler } => {
                write!(f, "Failed to send event to handler '{}'", handler)
            }
            EventBusError::AlreadyRegistered { handler } => {
                write!(f, "Handler '{}' is already registered", handler)
            }
        }
    }
}

impl std::error::Error for EventBusError {}

/// Result of sending an event to a handler
#[derive(Debug)]
pub struct SendResult {
    pub handler: String,
    pub result: Result<(), EventBusError>,
}

/// Event bus for routing events between handlers.
///
/// The EventBus maintains a registry of handler channels. Handlers register
/// to receive events, and can send events to other handlers by name.
///
/// # Thread Safety
///
/// EventBus is designed to be shared across multiple async tasks using `Arc<EventBus>`.
/// Internal synchronization uses `RwLock` to allow concurrent reads with exclusive writes.
///
/// # Example
///
/// ```ignore
/// let event_bus = Arc::new(EventBus::new());
///
/// // Register handlers
/// let rx1 = event_bus.register("handler-1").unwrap();
/// let rx2 = event_bus.register("handler-2").unwrap();
///
/// // Send an event from handler-1 to handler-2
/// let event = Event::from_filesystem("handler-1", "/tmp/test.txt", "create");
/// event_bus.send("handler-2", event).unwrap();
/// ```
pub struct EventBus {
    /// Map of handler names to their event senders
    senders: RwLock<HashMap<String, mpsc::UnboundedSender<Event>>>,
}

impl EventBus {
    /// Create a new empty EventBus
    pub fn new() -> Self {
        Self {
            senders: RwLock::new(HashMap::new()),
        }
    }

    /// Register a handler to receive events.
    ///
    /// Returns an unbounded receiver that the handler can use to receive forwarded events.
    ///
    /// # Errors
    ///
    /// Returns `EventBusError::AlreadyRegistered` if a handler with this name is already registered.
    pub fn register(
        &self,
        handler_id: &str,
    ) -> Result<mpsc::UnboundedReceiver<Event>, EventBusError> {
        let mut senders = self.senders.write().unwrap();

        if senders.contains_key(handler_id) {
            return Err(EventBusError::AlreadyRegistered {
                handler: handler_id.to_string(),
            });
        }

        let (tx, rx) = mpsc::unbounded_channel();
        senders.insert(handler_id.to_string(), tx);
        debug!("EventBus: Registered handler '{}'", handler_id);

        Ok(rx)
    }

    /// Unregister a handler from the event bus.
    ///
    /// After unregistration, events can no longer be sent to this handler.
    /// Returns true if the handler was registered and removed, false if not found.
    pub fn unregister(&self, handler_id: &str) -> bool {
        let mut senders = self.senders.write().unwrap();
        let removed = senders.remove(handler_id).is_some();
        if removed {
            debug!("EventBus: Unregistered handler '{}'", handler_id);
        }
        removed
    }

    /// Send an event to a specific handler.
    ///
    /// # Errors
    ///
    /// - `EventBusError::HandlerNotFound` if the target handler is not registered
    /// - `EventBusError::SendFailed` if the channel is closed (handler has stopped)
    pub fn send(&self, target_handler: &str, event: Event) -> Result<(), EventBusError> {
        let senders = self.senders.read().unwrap();

        match senders.get(target_handler) {
            Some(sender) => {
                sender.send(event).map_err(|_| {
                    warn!(
                        "EventBus: Failed to send event to '{}' - channel closed",
                        target_handler
                    );
                    EventBusError::SendFailed {
                        handler: target_handler.to_string(),
                    }
                })?;
                debug!("EventBus: Sent event to handler '{}'", target_handler);
                Ok(())
            }
            None => {
                warn!("EventBus: Target handler '{}' not found", target_handler);
                Err(EventBusError::HandlerNotFound {
                    handler: target_handler.to_string(),
                })
            }
        }
    }

    /// Send an event to multiple handlers.
    ///
    /// Returns a vector of results, one for each target handler.
    /// This allows callers to handle partial failures.
    pub fn send_many(&self, targets: &[String], event: Event) -> Vec<SendResult> {
        targets
            .iter()
            .map(|target| {
                let result = self.send(target, event.clone());
                SendResult {
                    handler: target.clone(),
                    result,
                }
            })
            .collect()
    }

    /// Check if a handler is registered
    pub fn is_registered(&self, handler_id: &str) -> bool {
        let senders = self.senders.read().unwrap();
        senders.contains_key(handler_id)
    }

    /// Get the number of registered handlers
    pub fn handler_count(&self) -> usize {
        let senders = self.senders.read().unwrap();
        senders.len()
    }

    /// Get the names of all registered handlers
    pub fn registered_handlers(&self) -> Vec<String> {
        let senders = self.senders.read().unwrap();
        senders.keys().cloned().collect()
    }
}

impl Default for EventBus {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::EventType;

    #[test]
    fn test_new_event_bus() {
        let bus = EventBus::new();
        assert_eq!(bus.handler_count(), 0);
    }

    #[test]
    fn test_register_handler() {
        let bus = EventBus::new();
        let result = bus.register("handler-1");
        assert!(result.is_ok());
        assert!(bus.is_registered("handler-1"));
        assert_eq!(bus.handler_count(), 1);
    }

    #[test]
    fn test_register_multiple_handlers() {
        let bus = EventBus::new();
        bus.register("handler-1").unwrap();
        bus.register("handler-2").unwrap();
        bus.register("handler-3").unwrap();

        assert_eq!(bus.handler_count(), 3);
        assert!(bus.is_registered("handler-1"));
        assert!(bus.is_registered("handler-2"));
        assert!(bus.is_registered("handler-3"));
    }

    #[test]
    fn test_register_duplicate_handler() {
        let bus = EventBus::new();
        bus.register("handler-1").unwrap();

        let result = bus.register("handler-1");
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            EventBusError::AlreadyRegistered { handler } if handler == "handler-1"
        ));
    }

    #[test]
    fn test_unregister_handler() {
        let bus = EventBus::new();
        bus.register("handler-1").unwrap();
        assert!(bus.is_registered("handler-1"));

        let removed = bus.unregister("handler-1");
        assert!(removed);
        assert!(!bus.is_registered("handler-1"));
        assert_eq!(bus.handler_count(), 0);
    }

    #[test]
    fn test_unregister_nonexistent_handler() {
        let bus = EventBus::new();
        let removed = bus.unregister("nonexistent");
        assert!(!removed);
    }

    #[tokio::test]
    async fn test_send_event() {
        let bus = EventBus::new();
        let mut rx = bus.register("handler-1").unwrap();

        let event = Event::new("source", EventType::Filesystem).with_data("test", "value");

        let send_result = bus.send("handler-1", event.clone());
        assert!(send_result.is_ok());

        // Receive the event
        let received = rx.recv().await;
        assert!(received.is_some());
        let received_event = received.unwrap();
        assert_eq!(received_event.id, event.id);
        assert_eq!(received_event.get("test"), Some(&"value".to_string()));
    }

    #[test]
    fn test_send_to_nonexistent_handler() {
        let bus = EventBus::new();
        let event = Event::new("source", EventType::Filesystem);

        let result = bus.send("nonexistent", event);
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            EventBusError::HandlerNotFound { handler } if handler == "nonexistent"
        ));
    }

    #[tokio::test]
    async fn test_send_to_closed_channel() {
        let bus = EventBus::new();
        let rx = bus.register("handler-1").unwrap();

        // Drop the receiver to close the channel
        drop(rx);

        let event = Event::new("source", EventType::Filesystem);
        let result = bus.send("handler-1", event);

        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            EventBusError::SendFailed { handler } if handler == "handler-1"
        ));
    }

    #[tokio::test]
    async fn test_send_many() {
        let bus = EventBus::new();
        let mut rx1 = bus.register("handler-1").unwrap();
        let mut rx2 = bus.register("handler-2").unwrap();

        let event = Event::new("source", EventType::Webhook);
        let targets = vec!["handler-1".to_string(), "handler-2".to_string()];

        let results = bus.send_many(&targets, event);

        assert_eq!(results.len(), 2);
        assert!(results.iter().all(|r| r.result.is_ok()));

        // Both handlers should receive the event
        let recv1 = rx1.recv().await;
        let recv2 = rx2.recv().await;
        assert!(recv1.is_some());
        assert!(recv2.is_some());
    }

    #[tokio::test]
    async fn test_send_many_partial_failure() {
        let bus = EventBus::new();
        let _rx1 = bus.register("handler-1").unwrap();
        // handler-2 is not registered

        let event = Event::new("source", EventType::Webhook);
        let targets = vec![
            "handler-1".to_string(),
            "handler-2".to_string(), // Not registered
        ];

        let results = bus.send_many(&targets, event);

        assert_eq!(results.len(), 2);
        assert!(results[0].result.is_ok());
        assert!(results[1].result.is_err());
    }

    #[test]
    fn test_registered_handlers() {
        let bus = EventBus::new();
        bus.register("alpha").unwrap();
        bus.register("beta").unwrap();
        bus.register("gamma").unwrap();

        let handlers = bus.registered_handlers();
        assert_eq!(handlers.len(), 3);
        assert!(handlers.contains(&"alpha".to_string()));
        assert!(handlers.contains(&"beta".to_string()));
        assert!(handlers.contains(&"gamma".to_string()));
    }

    #[test]
    fn test_event_bus_error_display() {
        assert_eq!(
            EventBusError::HandlerNotFound {
                handler: "foo".to_string()
            }
            .to_string(),
            "Handler 'foo' not found in event bus"
        );

        assert_eq!(
            EventBusError::SendFailed {
                handler: "bar".to_string()
            }
            .to_string(),
            "Failed to send event to handler 'bar'"
        );

        assert_eq!(
            EventBusError::AlreadyRegistered {
                handler: "baz".to_string()
            }
            .to_string(),
            "Handler 'baz' is already registered"
        );
    }

    #[tokio::test]
    async fn test_event_preserved_through_bus() {
        let bus = EventBus::new();
        let mut rx = bus.register("target").unwrap();

        let original = Event::from_filesystem("source-handler", "/path/to/file.txt", "modify")
            .with_data("extra", "data");

        bus.send("target", original.clone()).unwrap();

        let received = rx.recv().await.unwrap();

        // Verify all fields are preserved
        assert_eq!(received.id, original.id);
        assert_eq!(received.source_handler, original.source_handler);
        assert_eq!(received.event_type, original.event_type);
        assert_eq!(received.data, original.data);
    }

    #[test]
    fn test_default_impl() {
        let bus = EventBus::default();
        assert_eq!(bus.handler_count(), 0);
    }
}
