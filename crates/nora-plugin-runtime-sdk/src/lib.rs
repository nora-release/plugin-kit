//! A Rust server for the same JSON-RPC protocol other language plugins use.

pub mod transport;

pub use nora_plugin_protocol as protocol;

mod context;
pub mod provider;
mod server;
pub mod streams;

pub use context::PluginContext;
pub use server::{serve, serve_io, ServerOptions};

use protocol::{
    CallToolParams, InitializeParams, InitializeResult, RpcError, ServiceCallParams, ToolResult,
};
use serde_json::Value;
use std::future::Future;

pub trait Plugin: Send + Sync + 'static {
    fn initialize(
        &self,
        params: InitializeParams,
        context: PluginContext,
    ) -> impl Future<Output = Result<InitializeResult, RpcError>> + Send;

    fn call_tool(
        &self,
        params: CallToolParams,
        context: PluginContext,
    ) -> impl Future<Output = Result<ToolResult, RpcError>> + Send;

    fn call_service(
        &self,
        _params: ServiceCallParams,
        _context: PluginContext,
    ) -> impl Future<Output = Result<Value, RpcError>> + Send {
        std::future::ready(Err(RpcError::method_not_found()))
    }
}

#[cfg(test)]
#[path = "../test/sdk.test.rs"]
mod tests;
