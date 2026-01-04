use serde::{Deserialize, Serialize};
use super::RequestId;

pub type RpcResult = serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Response {
    pub id: RequestId,
    pub result: Option<RpcResult>,
    pub error: Option<String>,
    pub success: bool,
}

impl Response {
    pub fn success(id: RequestId, result: RpcResult) -> Self {
        Response {
            id,
            result: Some(result),
            error: None,
            success: true,
        }
    }

    pub fn error(id: RequestId, error: impl Into<String>) -> Self {
        Response {
            id,
            result: None,
            error: Some(error.into()),
            success: false,
        }
    }
}
