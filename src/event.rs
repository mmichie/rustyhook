//! Event types for inter-handler communication.
//!
//! This module provides a common Event struct that handlers use to communicate
//! with each other via the EventBus. Each handler type can create events with
//! type-specific data that gets passed to downstream handlers.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

/// Common event keys used across different event types
pub mod keys {
    // Common keys
    pub const SOURCE_HANDLER: &str = "source_handler";
    pub const EVENT_TYPE: &str = "event_type";

    // Filesystem event keys
    pub const FILE_PATH: &str = "file_path";
    pub const FILE_EVENT_KIND: &str = "file_event_kind";

    // Webhook event keys
    pub const HTTP_METHOD: &str = "http_method";
    pub const HTTP_URI: &str = "http_uri";
    pub const HTTP_BODY: &str = "http_body";
    pub const HTTP_HEADERS: &str = "http_headers";

    // SQS event keys
    pub const SQS_MESSAGE_BODY: &str = "sqs_message_body";
    pub const SQS_MESSAGE_ID: &str = "sqs_message_id";
    pub const SQS_RECEIPT_HANDLE: &str = "sqs_receipt_handle";

    // Cron event keys
    pub const CRON_EXPRESSION: &str = "cron_expression";
    pub const CRON_SCHEDULED_TIME: &str = "cron_scheduled_time";
}

/// Represents the type of event source
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EventType {
    Filesystem,
    Webhook,
    Sqs,
    Cron,
    Forwarded,
}

impl std::fmt::Display for EventType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EventType::Filesystem => write!(f, "filesystem"),
            EventType::Webhook => write!(f, "webhook"),
            EventType::Sqs => write!(f, "sqs"),
            EventType::Cron => write!(f, "cron"),
            EventType::Forwarded => write!(f, "forwarded"),
        }
    }
}

/// An event that can be passed between handlers via the EventBus.
///
/// Events contain metadata about their origin and a flexible key-value
/// data map that holds type-specific information.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Event {
    /// Unique identifier for this event (UUID v4)
    pub id: String,

    /// Name of the handler that originally created this event
    pub source_handler: String,

    /// Type of event source
    pub event_type: EventType,

    /// When the event was created
    pub timestamp: DateTime<Utc>,

    /// Type-specific event data as key-value pairs
    pub data: HashMap<String, String>,
}

impl Event {
    /// Create a new event with the given source and type
    pub fn new(source_handler: impl Into<String>, event_type: EventType) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            source_handler: source_handler.into(),
            event_type,
            timestamp: Utc::now(),
            data: HashMap::new(),
        }
    }

    /// Add a key-value pair to the event data
    pub fn with_data(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.data.insert(key.into(), value.into());
        self
    }

    /// Add multiple key-value pairs to the event data
    pub fn with_data_map(mut self, data: HashMap<String, String>) -> Self {
        self.data.extend(data);
        self
    }

    /// Create an event from a filesystem change
    pub fn from_filesystem(
        handler_name: impl Into<String>,
        path: impl Into<String>,
        event_kind: impl Into<String>,
    ) -> Self {
        Self::new(handler_name, EventType::Filesystem)
            .with_data(keys::FILE_PATH, path)
            .with_data(keys::FILE_EVENT_KIND, event_kind)
    }

    /// Create an event from a webhook request
    pub fn from_webhook(
        handler_name: impl Into<String>,
        method: impl Into<String>,
        uri: impl Into<String>,
        body: impl Into<String>,
    ) -> Self {
        Self::new(handler_name, EventType::Webhook)
            .with_data(keys::HTTP_METHOD, method)
            .with_data(keys::HTTP_URI, uri)
            .with_data(keys::HTTP_BODY, body)
    }

    /// Create an event from a webhook request with headers
    pub fn from_webhook_with_headers(
        handler_name: impl Into<String>,
        method: impl Into<String>,
        uri: impl Into<String>,
        body: impl Into<String>,
        headers: impl Into<String>,
    ) -> Self {
        Self::from_webhook(handler_name, method, uri, body).with_data(keys::HTTP_HEADERS, headers)
    }

    /// Create an event from an SQS message
    pub fn from_sqs(
        handler_name: impl Into<String>,
        message_body: impl Into<String>,
        message_id: impl Into<String>,
        receipt_handle: impl Into<String>,
    ) -> Self {
        Self::new(handler_name, EventType::Sqs)
            .with_data(keys::SQS_MESSAGE_BODY, message_body)
            .with_data(keys::SQS_MESSAGE_ID, message_id)
            .with_data(keys::SQS_RECEIPT_HANDLE, receipt_handle)
    }

    /// Create an event from a cron trigger
    pub fn from_cron(
        handler_name: impl Into<String>,
        expression: impl Into<String>,
        scheduled_time: DateTime<Utc>,
    ) -> Self {
        Self::new(handler_name, EventType::Cron)
            .with_data(keys::CRON_EXPRESSION, expression)
            .with_data(keys::CRON_SCHEDULED_TIME, scheduled_time.to_rfc3339())
    }

    /// Create a forwarded event that wraps an existing event
    ///
    /// The original event's data is preserved, and the event_type
    /// is changed to Forwarded while keeping the original source_handler.
    pub fn forwarded(original: &Event) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            source_handler: original.source_handler.clone(),
            event_type: EventType::Forwarded,
            timestamp: Utc::now(),
            data: original.data.clone(),
        }
    }

    /// Serialize the event to JSON
    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string(self)
    }

    /// Serialize the event to pretty-printed JSON
    pub fn to_json_pretty(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(self)
    }

    /// Get a data value by key
    pub fn get(&self, key: &str) -> Option<&String> {
        self.data.get(key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_event() {
        let event = Event::new("test-handler", EventType::Filesystem);

        assert!(!event.id.is_empty());
        assert_eq!(event.source_handler, "test-handler");
        assert_eq!(event.event_type, EventType::Filesystem);
        assert!(event.data.is_empty());
    }

    #[test]
    fn test_event_with_data() {
        let event = Event::new("test-handler", EventType::Webhook)
            .with_data("key1", "value1")
            .with_data("key2", "value2");

        assert_eq!(event.data.len(), 2);
        assert_eq!(event.get("key1"), Some(&"value1".to_string()));
        assert_eq!(event.get("key2"), Some(&"value2".to_string()));
    }

    #[test]
    fn test_from_filesystem() {
        let event = Event::from_filesystem("fs-handler", "/tmp/test.txt", "create");

        assert_eq!(event.source_handler, "fs-handler");
        assert_eq!(event.event_type, EventType::Filesystem);
        assert_eq!(
            event.get(keys::FILE_PATH),
            Some(&"/tmp/test.txt".to_string())
        );
        assert_eq!(
            event.get(keys::FILE_EVENT_KIND),
            Some(&"create".to_string())
        );
    }

    #[test]
    fn test_from_webhook() {
        let event = Event::from_webhook("webhook-handler", "POST", "/api/hook", r#"{"data": 1}"#);

        assert_eq!(event.source_handler, "webhook-handler");
        assert_eq!(event.event_type, EventType::Webhook);
        assert_eq!(event.get(keys::HTTP_METHOD), Some(&"POST".to_string()));
        assert_eq!(event.get(keys::HTTP_URI), Some(&"/api/hook".to_string()));
        assert_eq!(
            event.get(keys::HTTP_BODY),
            Some(&r#"{"data": 1}"#.to_string())
        );
    }

    #[test]
    fn test_from_webhook_with_headers() {
        let event = Event::from_webhook_with_headers(
            "webhook-handler",
            "POST",
            "/api/hook",
            "body",
            r#"{"Content-Type": "application/json"}"#,
        );

        assert_eq!(
            event.get(keys::HTTP_HEADERS),
            Some(&r#"{"Content-Type": "application/json"}"#.to_string())
        );
    }

    #[test]
    fn test_from_sqs() {
        let event = Event::from_sqs(
            "sqs-handler",
            r#"{"message": "hello"}"#,
            "msg-123",
            "receipt-456",
        );

        assert_eq!(event.source_handler, "sqs-handler");
        assert_eq!(event.event_type, EventType::Sqs);
        assert_eq!(
            event.get(keys::SQS_MESSAGE_BODY),
            Some(&r#"{"message": "hello"}"#.to_string())
        );
        assert_eq!(
            event.get(keys::SQS_MESSAGE_ID),
            Some(&"msg-123".to_string())
        );
        assert_eq!(
            event.get(keys::SQS_RECEIPT_HANDLE),
            Some(&"receipt-456".to_string())
        );
    }

    #[test]
    fn test_from_cron() {
        let scheduled = Utc::now();
        let event = Event::from_cron("cron-handler", "0 * * * * *", scheduled);

        assert_eq!(event.source_handler, "cron-handler");
        assert_eq!(event.event_type, EventType::Cron);
        assert_eq!(
            event.get(keys::CRON_EXPRESSION),
            Some(&"0 * * * * *".to_string())
        );
        assert_eq!(
            event.get(keys::CRON_SCHEDULED_TIME),
            Some(&scheduled.to_rfc3339())
        );
    }

    #[test]
    fn test_forwarded_event() {
        let original = Event::from_filesystem("fs-handler", "/tmp/test.txt", "modify");
        let forwarded = Event::forwarded(&original);

        // New ID but same source
        assert_ne!(forwarded.id, original.id);
        assert_eq!(forwarded.source_handler, original.source_handler);

        // Type changed to Forwarded
        assert_eq!(forwarded.event_type, EventType::Forwarded);

        // Data preserved
        assert_eq!(forwarded.data, original.data);
    }

    #[test]
    fn test_event_to_json() {
        let event = Event::from_filesystem("fs-handler", "/tmp/test.txt", "create");
        let json = event.to_json().expect("Failed to serialize to JSON");

        assert!(json.contains("fs-handler"));
        assert!(json.contains("filesystem"));
        assert!(json.contains("/tmp/test.txt"));
    }

    #[test]
    fn test_event_deserialization() {
        let event = Event::from_webhook("test", "GET", "/path", "body");
        let json = event.to_json().expect("Failed to serialize");

        let deserialized: Event = serde_json::from_str(&json).expect("Failed to deserialize");

        assert_eq!(deserialized.id, event.id);
        assert_eq!(deserialized.source_handler, event.source_handler);
        assert_eq!(deserialized.event_type, event.event_type);
        assert_eq!(deserialized.data, event.data);
    }

    #[test]
    fn test_event_type_display() {
        assert_eq!(EventType::Filesystem.to_string(), "filesystem");
        assert_eq!(EventType::Webhook.to_string(), "webhook");
        assert_eq!(EventType::Sqs.to_string(), "sqs");
        assert_eq!(EventType::Cron.to_string(), "cron");
        assert_eq!(EventType::Forwarded.to_string(), "forwarded");
    }

    #[test]
    fn test_with_data_map() {
        let mut data = HashMap::new();
        data.insert("key1".to_string(), "value1".to_string());
        data.insert("key2".to_string(), "value2".to_string());

        let event = Event::new("handler", EventType::Webhook).with_data_map(data);

        assert_eq!(event.data.len(), 2);
        assert_eq!(event.get("key1"), Some(&"value1".to_string()));
    }

    #[test]
    fn test_unique_event_ids() {
        let event1 = Event::new("handler", EventType::Filesystem);
        let event2 = Event::new("handler", EventType::Filesystem);

        assert_ne!(event1.id, event2.id);
    }
}
