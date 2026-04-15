//! BSV overlay integration — agent registration and generic lookup.
//!
//! The overlay is a discovery layer. Agents publish 6-field AGENT PushDrop tokens
//! to make themselves discoverable by capability, identity key, or endpoint.
//! Identity and capabilities flow from the BRC-52 certificate into the registration.
//!
//! Two operations:
//! - **Registration** (startup lifecycle): build AGENT PushDrop from cert, submit BEEF
//! - **Lookup** (tool): query any overlay lookup service, return structured records

pub mod lookup;
pub mod registration;
pub mod reregister;

pub use lookup::{overlay_lookup, AgentRecord};
pub use registration::{check_registered, deregister_from_overlay, register_on_overlay};
pub use reregister::{reregister_on_overlay, ReregistrationResult};

/// Well-known BRC-46 basket for overlay agent registration tokens.
pub const BASKET_AGENT_REGISTRATION: &str = "dm-agent-registration";

/// BRC-42 protocol ID for agent registry signatures.
pub fn agent_registry_protocol_id() -> serde_json::Value {
    serde_json::json!([2, "agent registry"])
}

/// BRC-42 key ID for agent registry.
pub const AGENT_REGISTRY_KEY_ID: &str = "1";

/// BRC-42 counterparty for agent registry (publicly verifiable).
pub const AGENT_REGISTRY_COUNTERPARTY: &str = "anyone";
