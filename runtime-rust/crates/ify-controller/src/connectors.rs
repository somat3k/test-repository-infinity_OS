//! Workflow node connector templates — Epic F item 10.
//!
//! This module defines the set of built-in connector node templates for
//! workflow automation.  Each connector exposes a canonical set of typed
//! input/output ports and default parameters, enabling the canvas to
//! instantiate ready-to-use workflow nodes without manual port definition.
//!
//! ## Connector catalogue
//!
//! | Kind | `node_kind` string |
//! |------|--------------------|
//! | HTTP request | `http.request` |
//! | Webhook receiver | `http.webhook` |
//! | Blockchain RPC | `blockchain.rpc` |
//! | Blockchain sign / submit | `blockchain.sign` |
//! | Database query | `db.query` |
//! | Database write | `db.write` |
//! | ML predict | `ml.predict` |
//! | ML train | `ml.train` |
//! | Trading order | `trading.order` |
//! | Market data | `trading.market_data` |

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::graph::{GraphNode, PortDataType, PortDef, PortDirection};

// ---------------------------------------------------------------------------
// ConnectorKind
// ---------------------------------------------------------------------------

/// Identifies the category of a workflow connector.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConnectorKind {
    /// HTTP GET / POST / PUT / DELETE / PATCH request.
    HttpRequest,
    /// Incoming HTTP webhook receiver.
    HttpWebhook,
    /// Blockchain JSON-RPC call.
    BlockchainRpc,
    /// Blockchain transaction signing and submission.
    BlockchainSign,
    /// SQL or NoSQL database query.
    DatabaseQuery,
    /// SQL or NoSQL database write (insert / update / delete).
    DatabaseWrite,
    /// ML model inference (predict).
    MlPredict,
    /// ML model training job.
    MlTrain,
    /// Trading order submission (market / limit / stop).
    TradingOrder,
    /// Market data ingestion and normalisation.
    TradingMarketData,
}

impl ConnectorKind {
    /// Canonical `node_kind` string used in [`crate::graph::GraphNode::kind`].
    pub fn node_kind(self) -> &'static str {
        match self {
            Self::HttpRequest => "http.request",
            Self::HttpWebhook => "http.webhook",
            Self::BlockchainRpc => "blockchain.rpc",
            Self::BlockchainSign => "blockchain.sign",
            Self::DatabaseQuery => "db.query",
            Self::DatabaseWrite => "db.write",
            Self::MlPredict => "ml.predict",
            Self::MlTrain => "ml.train",
            Self::TradingOrder => "trading.order",
            Self::TradingMarketData => "trading.market_data",
        }
    }

    /// Human-readable display name.
    pub fn display_name(self) -> &'static str {
        match self {
            Self::HttpRequest => "HTTP Request",
            Self::HttpWebhook => "HTTP Webhook",
            Self::BlockchainRpc => "Blockchain RPC",
            Self::BlockchainSign => "Blockchain Sign & Submit",
            Self::DatabaseQuery => "Database Query",
            Self::DatabaseWrite => "Database Write",
            Self::MlPredict => "ML Predict",
            Self::MlTrain => "ML Train",
            Self::TradingOrder => "Trading Order",
            Self::TradingMarketData => "Market Data",
        }
    }
}

// ---------------------------------------------------------------------------
// ConnectorParam — default parameter descriptor
// ---------------------------------------------------------------------------

/// A default parameter descriptor for a connector template.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectorParam {
    /// Parameter name (used as key in [`crate::graph::GraphNode::parameters`]).
    pub name: &'static str,
    /// Human-readable description.
    pub description: &'static str,
    /// Whether the parameter must be provided before execution.
    pub required: bool,
    /// Default value; `None` means the user must supply one.
    pub default: Option<serde_json::Value>,
}

impl ConnectorParam {
    fn required(name: &'static str, desc: &'static str) -> Self {
        Self { name, description: desc, required: true, default: None }
    }

    fn optional(name: &'static str, desc: &'static str, default: serde_json::Value) -> Self {
        Self { name, description: desc, required: false, default: Some(default) }
    }
}

// ---------------------------------------------------------------------------
// ConnectorTemplate
// ---------------------------------------------------------------------------

/// A fully-specified template for instantiating a workflow connector node.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectorTemplate {
    /// Connector category.
    pub kind: ConnectorKind,
    /// `node_kind` identifier (mirrors [`ConnectorKind::node_kind`]).
    pub node_kind: &'static str,
    /// Human-readable display name.
    pub display_name: &'static str,
    /// Brief description of what this connector does.
    pub description: &'static str,
    /// Port definitions for the template.
    ///
    /// These are *cloned* into a new [`GraphNode`] when [`ConnectorTemplate::instantiate`]
    /// is called.
    pub ports: Vec<PortDef>,
    /// Default parameters.
    pub params: Vec<ConnectorParam>,
}

impl ConnectorTemplate {
    /// Instantiate a new [`GraphNode`] from this template.
    ///
    /// The returned node has all template ports attached and all default
    /// parameters pre-filled.  The caller should set `.provenance` and
    /// `.position` before inserting into a graph.
    pub fn instantiate(&self, label: impl Into<String>) -> GraphNode {
        let mut node = GraphNode::new(self.node_kind, label);
        for port in &self.ports {
            // Ignore duplicate-name errors — templates are well-defined.
            let _ = node.add_port(port.clone());
        }
        for param in &self.params {
            if let Some(default) = &param.default {
                node.parameters.insert(param.name.to_owned(), default.clone());
            }
        }
        node
    }
}

// ---------------------------------------------------------------------------
// ConnectorRegistry
// ---------------------------------------------------------------------------

/// Registry of all built-in workflow connector templates.
///
/// Use [`ConnectorRegistry::new`] to obtain a registry pre-populated with all
/// built-in templates, then [`ConnectorRegistry::get`] to retrieve a template
/// by [`ConnectorKind`].
pub struct ConnectorRegistry {
    templates: HashMap<ConnectorKind, ConnectorTemplate>,
}

impl ConnectorRegistry {
    /// Create a registry containing all built-in connector templates.
    pub fn new() -> Self {
        let mut reg = Self { templates: HashMap::new() };
        for t in built_in_templates() {
            reg.templates.insert(t.kind, t);
        }
        reg
    }

    /// Return the template for the given [`ConnectorKind`], if registered.
    pub fn get(&self, kind: ConnectorKind) -> Option<&ConnectorTemplate> {
        self.templates.get(&kind)
    }

    /// Return an iterator over all registered templates.
    pub fn all(&self) -> impl Iterator<Item = &ConnectorTemplate> {
        self.templates.values()
    }

    /// Instantiate a new node from the given connector kind.
    ///
    /// Returns `None` if the kind is not registered.
    pub fn instantiate(
        &self,
        kind: ConnectorKind,
        label: impl Into<String>,
    ) -> Option<GraphNode> {
        self.get(kind).map(|t| t.instantiate(label))
    }
}

impl Default for ConnectorRegistry {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Built-in template definitions
// ---------------------------------------------------------------------------

fn built_in_templates() -> Vec<ConnectorTemplate> {
    vec![
        // ── HTTP Request ──────────────────────────────────────────────────
        ConnectorTemplate {
            kind: ConnectorKind::HttpRequest,
            node_kind: "http.request",
            display_name: "HTTP Request",
            description: "Send an HTTP request and receive the response.",
            ports: vec![
                PortDef::new("trigger", PortDirection::In, PortDataType::Any)
                    .with_description("Execution trigger"),
                PortDef::new("body", PortDirection::In, PortDataType::Json)
                    .with_description("Optional request body"),
                PortDef::new("response", PortDirection::Out, PortDataType::Json)
                    .with_description("Parsed JSON response"),
                PortDef::new("status_code", PortDirection::Out, PortDataType::Number)
                    .with_description("HTTP response status code"),
                PortDef::new("error", PortDirection::Out, PortDataType::String)
                    .with_description("Error message if request failed"),
            ],
            params: vec![
                ConnectorParam::required("url", "Request URL"),
                ConnectorParam::optional("method", "HTTP method", serde_json::json!("GET")),
                ConnectorParam::optional("timeout_ms", "Request timeout in milliseconds", serde_json::json!(5000)),
                ConnectorParam::optional("headers", "Additional request headers (JSON object)", serde_json::json!({})),
            ],
        },

        // ── HTTP Webhook ──────────────────────────────────────────────────
        ConnectorTemplate {
            kind: ConnectorKind::HttpWebhook,
            node_kind: "http.webhook",
            display_name: "HTTP Webhook",
            description: "Receive incoming HTTP webhook events.",
            ports: vec![
                PortDef::new("payload", PortDirection::Out, PortDataType::Json)
                    .with_description("Incoming webhook payload"),
                PortDef::new("headers", PortDirection::Out, PortDataType::Json)
                    .with_description("Incoming HTTP headers"),
                PortDef::new("method", PortDirection::Out, PortDataType::String)
                    .with_description("HTTP method of the incoming request"),
            ],
            params: vec![
                ConnectorParam::required("path", "Webhook path (e.g. /hooks/my-event)"),
                ConnectorParam::optional("secret", "HMAC signing secret for payload verification", serde_json::json!("")),
            ],
        },

        // ── Blockchain RPC ────────────────────────────────────────────────
        ConnectorTemplate {
            kind: ConnectorKind::BlockchainRpc,
            node_kind: "blockchain.rpc",
            display_name: "Blockchain RPC",
            description: "Execute a JSON-RPC call against a blockchain node.",
            ports: vec![
                PortDef::new("trigger", PortDirection::In, PortDataType::Any)
                    .with_description("Execution trigger"),
                PortDef::new("params", PortDirection::In, PortDataType::Array)
                    .with_description("RPC method parameters array"),
                PortDef::new("result", PortDirection::Out, PortDataType::Json)
                    .with_description("RPC result value"),
                PortDef::new("error", PortDirection::Out, PortDataType::String)
                    .with_description("RPC error message, if any"),
            ],
            params: vec![
                ConnectorParam::required("rpc_url", "Blockchain node RPC endpoint URL"),
                ConnectorParam::required("method", "JSON-RPC method name (e.g. eth_blockNumber)"),
                ConnectorParam::optional("chain_id", "EVM chain ID", serde_json::json!(1)),
            ],
        },

        // ── Blockchain Sign ───────────────────────────────────────────────
        ConnectorTemplate {
            kind: ConnectorKind::BlockchainSign,
            node_kind: "blockchain.sign",
            display_name: "Blockchain Sign & Submit",
            description: "Sign and submit a transaction to a blockchain.",
            ports: vec![
                PortDef::new("tx_data", PortDirection::In, PortDataType::Json)
                    .with_description("Unsigned transaction object")
                    .required(),
                PortDef::new("tx_hash", PortDirection::Out, PortDataType::String)
                    .with_description("Submitted transaction hash"),
                PortDef::new("receipt", PortDirection::Out, PortDataType::Json)
                    .with_description("Transaction receipt (after confirmation)"),
                PortDef::new("error", PortDirection::Out, PortDataType::String)
                    .with_description("Error message if submission failed"),
            ],
            params: vec![
                ConnectorParam::required("rpc_url", "Blockchain node RPC endpoint URL"),
                ConnectorParam::required("key_ref", "Reference to the signing key (secret store key name)"),
                ConnectorParam::optional("wait_confirmations", "Number of confirmations to wait for receipt", serde_json::json!(1)),
            ],
        },

        // ── Database Query ────────────────────────────────────────────────
        ConnectorTemplate {
            kind: ConnectorKind::DatabaseQuery,
            node_kind: "db.query",
            display_name: "Database Query",
            description: "Execute a read query against a database.",
            ports: vec![
                PortDef::new("trigger", PortDirection::In, PortDataType::Any)
                    .with_description("Execution trigger"),
                PortDef::new("bindings", PortDirection::In, PortDataType::Json)
                    .with_description("Query parameter bindings"),
                PortDef::new("rows", PortDirection::Out, PortDataType::Array)
                    .with_description("Result rows as a JSON array"),
                PortDef::new("row_count", PortDirection::Out, PortDataType::Number)
                    .with_description("Number of rows returned"),
                PortDef::new("error", PortDirection::Out, PortDataType::String)
                    .with_description("Error message if query failed"),
            ],
            params: vec![
                ConnectorParam::required("connection_ref", "DB connection string reference"),
                ConnectorParam::required("query", "SQL or NoSQL query string"),
                ConnectorParam::optional("timeout_ms", "Query timeout in milliseconds", serde_json::json!(10000)),
            ],
        },

        // ── Database Write ────────────────────────────────────────────────
        ConnectorTemplate {
            kind: ConnectorKind::DatabaseWrite,
            node_kind: "db.write",
            display_name: "Database Write",
            description: "Execute a write (insert / update / delete) against a database.",
            ports: vec![
                PortDef::new("trigger", PortDirection::In, PortDataType::Any)
                    .with_description("Execution trigger"),
                PortDef::new("data", PortDirection::In, PortDataType::Json)
                    .with_description("Data payload to write"),
                PortDef::new("affected_rows", PortDirection::Out, PortDataType::Number)
                    .with_description("Number of rows affected"),
                PortDef::new("error", PortDirection::Out, PortDataType::String)
                    .with_description("Error message if write failed"),
            ],
            params: vec![
                ConnectorParam::required("connection_ref", "DB connection string reference"),
                ConnectorParam::required("statement", "SQL or NoSQL write statement"),
                ConnectorParam::optional("timeout_ms", "Write timeout in milliseconds", serde_json::json!(10000)),
            ],
        },

        // ── ML Predict ────────────────────────────────────────────────────
        ConnectorTemplate {
            kind: ConnectorKind::MlPredict,
            node_kind: "ml.predict",
            display_name: "ML Predict",
            description: "Run inference against a registered ML model.",
            ports: vec![
                PortDef::new("features", PortDirection::In, PortDataType::Json)
                    .with_description("Feature vector / input JSON")
                    .required(),
                PortDef::new("prediction", PortDirection::Out, PortDataType::Json)
                    .with_description("Model prediction output"),
                PortDef::new("confidence", PortDirection::Out, PortDataType::Number)
                    .with_description("Confidence or probability score [0,1]"),
                PortDef::new("error", PortDirection::Out, PortDataType::String)
                    .with_description("Inference error, if any"),
            ],
            params: vec![
                ConnectorParam::required("model_id", "Registered model identifier"),
                ConnectorParam::optional("version", "Model version (defaults to latest)", serde_json::json!("latest")),
                ConnectorParam::optional("timeout_ms", "Inference timeout in milliseconds", serde_json::json!(3000)),
            ],
        },

        // ── ML Train ─────────────────────────────────────────────────────
        ConnectorTemplate {
            kind: ConnectorKind::MlTrain,
            node_kind: "ml.train",
            display_name: "ML Train",
            description: "Submit a training job for an ML model.",
            ports: vec![
                PortDef::new("dataset_ref", PortDirection::In, PortDataType::String)
                    .with_description("Artifact ID of the training dataset")
                    .required(),
                PortDef::new("config", PortDirection::In, PortDataType::Json)
                    .with_description("Training configuration overrides"),
                PortDef::new("model_artifact", PortDirection::Out, PortDataType::String)
                    .with_description("Artifact ID of the trained model"),
                PortDef::new("metrics", PortDirection::Out, PortDataType::Json)
                    .with_description("Training metrics (loss, accuracy, etc.)"),
                PortDef::new("error", PortDirection::Out, PortDataType::String)
                    .with_description("Training error, if any"),
            ],
            params: vec![
                ConnectorParam::required("model_kind", "Type of model to train (e.g. xgboost, transformer)"),
                ConnectorParam::optional("max_epochs", "Maximum number of training epochs", serde_json::json!(100)),
                ConnectorParam::optional("gpu_budget", "GPU memory budget in MiB (0 = CPU only)", serde_json::json!(0)),
            ],
        },

        // ── Trading Order ─────────────────────────────────────────────────
        ConnectorTemplate {
            kind: ConnectorKind::TradingOrder,
            node_kind: "trading.order",
            display_name: "Trading Order",
            description: "Submit a trading order (market, limit, or stop).",
            ports: vec![
                PortDef::new("signal", PortDirection::In, PortDataType::Json)
                    .with_description("Trading signal including side, size, and price")
                    .required(),
                PortDef::new("order_id", PortDirection::Out, PortDataType::String)
                    .with_description("Exchange-assigned order ID"),
                PortDef::new("fill", PortDirection::Out, PortDataType::Json)
                    .with_description("Fill details (price, quantity, timestamp)"),
                PortDef::new("error", PortDirection::Out, PortDataType::String)
                    .with_description("Order rejection or error"),
            ],
            params: vec![
                ConnectorParam::required("exchange_ref", "Exchange connection reference"),
                ConnectorParam::required("symbol", "Trading symbol (e.g. BTC-USD)"),
                ConnectorParam::optional("order_type", "Order type: market | limit | stop", serde_json::json!("market")),
                ConnectorParam::optional("paper_mode", "Run in paper trading mode", serde_json::json!(true)),
            ],
        },

        // ── Market Data ───────────────────────────────────────────────────
        ConnectorTemplate {
            kind: ConnectorKind::TradingMarketData,
            node_kind: "trading.market_data",
            display_name: "Market Data",
            description: "Ingest and normalise market data (OHLCV, tick, order book).",
            ports: vec![
                PortDef::new("trigger", PortDirection::In, PortDataType::Any)
                    .with_description("Fetch trigger or subscription start"),
                PortDef::new("ohlcv", PortDirection::Out, PortDataType::Array)
                    .with_description("OHLCV candle array"),
                PortDef::new("last_price", PortDirection::Out, PortDataType::Number)
                    .with_description("Latest traded price"),
                PortDef::new("order_book", PortDirection::Out, PortDataType::Json)
                    .with_description("Current order book snapshot"),
                PortDef::new("error", PortDirection::Out, PortDataType::String)
                    .with_description("Data fetch error, if any"),
            ],
            params: vec![
                ConnectorParam::required("exchange_ref", "Exchange connection reference"),
                ConnectorParam::required("symbol", "Trading symbol (e.g. ETH-USD)"),
                ConnectorParam::optional("interval", "Candle interval (e.g. 1m, 5m, 1h)", serde_json::json!("1m")),
                ConnectorParam::optional("limit", "Number of candles to fetch", serde_json::json!(100)),
            ],
        },
    ]
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_contains_all_kinds() {
        let reg = ConnectorRegistry::new();
        let kinds = [
            ConnectorKind::HttpRequest,
            ConnectorKind::HttpWebhook,
            ConnectorKind::BlockchainRpc,
            ConnectorKind::BlockchainSign,
            ConnectorKind::DatabaseQuery,
            ConnectorKind::DatabaseWrite,
            ConnectorKind::MlPredict,
            ConnectorKind::MlTrain,
            ConnectorKind::TradingOrder,
            ConnectorKind::TradingMarketData,
        ];
        for kind in kinds {
            assert!(
                reg.get(kind).is_some(),
                "missing template for {kind:?}"
            );
        }
    }

    #[test]
    fn instantiate_http_request_node() {
        let reg = ConnectorRegistry::new();
        let node = reg.instantiate(ConnectorKind::HttpRequest, "My HTTP Call").unwrap();
        assert_eq!(node.kind, "http.request");
        assert_eq!(node.label, "My HTTP Call");
        // Should have the default method parameter pre-filled.
        assert_eq!(node.parameters.get("method"), Some(&serde_json::json!("GET")));
        // Should have at least the known ports.
        assert!(node.ports.values().any(|p| p.name == "response"));
        assert!(node.ports.values().any(|p| p.name == "trigger"));
    }

    #[test]
    fn instantiate_ml_predict_node() {
        let reg = ConnectorRegistry::new();
        let node = reg.instantiate(ConnectorKind::MlPredict, "Inference").unwrap();
        assert_eq!(node.kind, "ml.predict");
        // Required input port for features.
        let feat_port = node.ports.values().find(|p| p.name == "features").unwrap();
        assert!(port_is_required(feat_port));
    }

    fn port_is_required(p: &crate::graph::PortDef) -> bool {
        p.required
    }

    #[test]
    fn connector_kind_node_kind_strings_unique() {
        let kinds = [
            ConnectorKind::HttpRequest,
            ConnectorKind::HttpWebhook,
            ConnectorKind::BlockchainRpc,
            ConnectorKind::BlockchainSign,
            ConnectorKind::DatabaseQuery,
            ConnectorKind::DatabaseWrite,
            ConnectorKind::MlPredict,
            ConnectorKind::MlTrain,
            ConnectorKind::TradingOrder,
            ConnectorKind::TradingMarketData,
        ];
        let strings: Vec<&str> = kinds.iter().map(|k| k.node_kind()).collect();
        let unique: std::collections::HashSet<&&str> = strings.iter().collect();
        assert_eq!(strings.len(), unique.len(), "all node_kind strings must be unique");
    }

    #[test]
    fn all_templates_have_at_least_one_port() {
        let reg = ConnectorRegistry::new();
        for t in reg.all() {
            assert!(!t.ports.is_empty(), "template {} has no ports", t.node_kind);
        }
    }
}
