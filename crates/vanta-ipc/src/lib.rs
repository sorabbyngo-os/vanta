//! Typed IPC protocol shared by Vanta GUI, shortcuts, and AION.
use serde::{Deserialize, Serialize};

/// Current IPC protocol version.
pub const PROTOCOL_VERSION: u16 = 1;

/// A request sent to the Vanta compositor.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Request {
    pub protocol_version: u16,
    pub request_id: u64,
    pub client_id: String,
    pub action: Action,
}

/// Maximum permitted client identifier length in bytes.
pub const MAX_CLIENT_ID_LEN: usize = 64;

/// Maximum permitted intent text length in bytes.
pub const MAX_INTENT_TEXT_LEN: usize = 4096;

/// Request validation failures.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ValidationError {
    UnsupportedProtocolVersion { received: u16 },
    ZeroRequestId,
    InvalidClientId,
    IntentTextEmpty,
    IntentTextTooLong,
}

impl Request {
    /// Validate protocol-level request invariants.
    pub fn validate(&self) -> Result<(), ValidationError> {
        if self.protocol_version != PROTOCOL_VERSION {
            return Err(ValidationError::UnsupportedProtocolVersion {
                received: self.protocol_version,
            });
        }

        if self.request_id == 0 {
            return Err(ValidationError::ZeroRequestId);
        }

        if self.client_id.trim().is_empty() || self.client_id.len() > MAX_CLIENT_ID_LEN {
            return Err(ValidationError::InvalidClientId);
        }

        if let Action::RequestIntent { text } = &self.action {
            if text.trim().is_empty() {
                return Err(ValidationError::IntentTextEmpty);
            }

            if text.len() > MAX_INTENT_TEXT_LEN {
                return Err(ValidationError::IntentTextTooLong);
            }
        }

        Ok(())
    }
}

/// Execution state of a request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum RequestStatus {
    /// The request passed initial validation and was queued.
    Accepted,
    /// The requested operation finished successfully.
    Completed,
    /// The request was refused before execution.
    Rejected,
    /// Execution failed.
    Failed,
}

/// Response sent by the compositor.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Response {
    pub protocol_version: u16,
    pub request_id: u64,
    pub status: RequestStatus,
    pub result: Option<ActionResult>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Action {
    FocusWindow {
        id: u64,
    },
    CloseWindow {
        id: u64,
    },
    ToggleFloat {
        id: u64,
    },
    MinimizeWindow {
        id: u64,
    },
    RestoreWindow {
        id: u64,
    },
    ListMinimizedWindows,
    ListWorkspaces,
    GetOverviewData,
    CreateWorkspace,
    SwitchWorkspace {
        index: u32,
    },

    /// Intent path — executed only after AION mediation.
    RequestIntent {
        text: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MinimizedWindowInfo {
    pub id: u64,
    pub title: Option<String>,
    pub app_id: Option<String>,
}

/// A live or minimized window represented in an overview snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OverviewWindowInfo {
    pub id: u64,
    pub title: Option<String>,
    pub app_id: Option<String>,
    pub minimized: bool,
}

/// Window metadata for one workspace, including empty workspaces.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OverviewWorkspaceInfo {
    pub index: u32,
    pub windows: Vec<OverviewWindowInfo>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ActionResult {
    Ok,
    MinimizedWindows {
        windows: Vec<MinimizedWindowInfo>,
    },
    Workspaces {
        count: u32,
        active_index: u32,
    },
    OverviewData {
        workspaces: Vec<OverviewWorkspaceInfo>,
        active_index: u32,
    },
    NeedConfirm {
        reason: String,
    },
    Err {
        message: String,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_round_trip() {
        let request = Request {
            protocol_version: PROTOCOL_VERSION,
            request_id: 1001,
            client_id: "vanta-shell".to_owned(),
            action: Action::FocusWindow { id: 42 },
        };

        let json = serde_json::to_string(&request).unwrap();
        let decoded: Request = serde_json::from_str(&json).unwrap();

        assert_eq!(decoded, request);
    }

    #[test]
    fn response_round_trip() {
        let response = Response {
            protocol_version: PROTOCOL_VERSION,
            request_id: 1001,
            status: RequestStatus::Completed,
            result: Some(ActionResult::Ok),
        };

        let json = serde_json::to_string(&response).unwrap();
        let decoded: Response = serde_json::from_str(&json).unwrap();

        assert_eq!(decoded, response);
    }

    #[test]
    fn accepted_response_has_no_result() {
        let response = Response {
            protocol_version: PROTOCOL_VERSION,
            request_id: 1002,
            status: RequestStatus::Accepted,
            result: None,
        };

        let json = serde_json::to_string(&response).unwrap();
        let decoded: Response = serde_json::from_str(&json).unwrap();

        assert_eq!(decoded, response);
    }

    #[test]
    fn request_preserves_intent_text() {
        let request = Request {
            protocol_version: PROTOCOL_VERSION,
            request_id: 1003,
            client_id: "aion".to_owned(),
            action: Action::RequestIntent {
                text: "Open workspace two".to_owned(),
            },
        };

        let json = serde_json::to_string(&request).unwrap();
        let decoded: Request = serde_json::from_str(&json).unwrap();

        assert_eq!(decoded, request);
    }

    #[test]
    fn rejects_malformed_request_json() {
        assert!(serde_json::from_str::<Request>("{invalid json}").is_err());
    }

    #[test]
    fn rejects_unknown_action_variant() {
        let json = r#"{"UnknownAction":{"id":42}}"#;
        assert!(serde_json::from_str::<Action>(json).is_err());
    }
}

#[cfg(test)]
mod action_tests {
    use super::*;

    #[test]
    fn all_actions_round_trip() {
        let actions = [
            Action::FocusWindow { id: 42 },
            Action::CloseWindow { id: 7 },
            Action::ToggleFloat { id: 9 },
            Action::MinimizeWindow { id: 9 },
            Action::RestoreWindow { id: 9 },
            Action::ListMinimizedWindows,
            Action::ListWorkspaces,
            Action::GetOverviewData,
            Action::CreateWorkspace,
            Action::SwitchWorkspace { index: 3 },
            Action::RequestIntent {
                text: "Open workspace two".to_owned(),
            },
        ];

        for action in actions {
            let json = serde_json::to_string(&action).unwrap();
            let decoded: Action = serde_json::from_str(&json).unwrap();
            assert_eq!(decoded, action);
        }
    }

    #[test]
    fn all_results_round_trip() {
        let results = [
            ActionResult::Ok,
            ActionResult::MinimizedWindows {
                windows: vec![MinimizedWindowInfo {
                    id: 42,
                    title: Some("Terminal".to_owned()),
                    app_id: Some("org.example.Terminal".to_owned()),
                }],
            },
            ActionResult::Workspaces {
                count: 4,
                active_index: 2,
            },
            ActionResult::OverviewData {
                workspaces: vec![
                    OverviewWorkspaceInfo {
                        index: 0,
                        windows: vec![OverviewWindowInfo {
                            id: 42,
                            title: Some("Terminal".to_owned()),
                            app_id: Some("org.example.Terminal".to_owned()),
                            minimized: false,
                        }],
                    },
                    OverviewWorkspaceInfo {
                        index: 1,
                        windows: Vec::new(),
                    },
                ],
                active_index: 0,
            },
            ActionResult::NeedConfirm {
                reason: "Closing this window".to_owned(),
            },
            ActionResult::Err {
                message: "Window not found".to_owned(),
            },
        ];

        for result in results {
            let json = serde_json::to_string(&result).unwrap();
            let decoded: ActionResult = serde_json::from_str(&json).unwrap();
            assert_eq!(decoded, result);
        }
    }
}

#[cfg(test)]
mod validation_tests {
    use super::*;

    fn valid_request(action: Action) -> Request {
        Request {
            protocol_version: PROTOCOL_VERSION,
            request_id: 1,
            client_id: "vanta-shell".to_owned(),
            action,
        }
    }

    #[test]
    fn accepts_valid_request() {
        assert!(valid_request(Action::FocusWindow { id: 42 })
            .validate()
            .is_ok());
    }

    #[test]
    fn rejects_unsupported_protocol_version() {
        let mut request = valid_request(Action::FocusWindow { id: 42 });
        request.protocol_version = 99;

        assert!(matches!(
            request.validate(),
            Err(ValidationError::UnsupportedProtocolVersion { received: 99 })
        ));
    }

    #[test]
    fn rejects_zero_request_id() {
        let mut request = valid_request(Action::FocusWindow { id: 42 });
        request.request_id = 0;

        assert_eq!(request.validate(), Err(ValidationError::ZeroRequestId));
    }

    #[test]
    fn rejects_empty_client_id() {
        let mut request = valid_request(Action::FocusWindow { id: 42 });
        request.client_id = "  ".to_owned();

        assert_eq!(request.validate(), Err(ValidationError::InvalidClientId));
    }

    #[test]
    fn rejects_empty_intent() {
        let request = valid_request(Action::RequestIntent {
            text: "   ".to_owned(),
        });

        assert_eq!(request.validate(), Err(ValidationError::IntentTextEmpty));
    }

    #[test]
    fn rejects_oversized_intent() {
        let request = valid_request(Action::RequestIntent {
            text: "x".repeat(MAX_INTENT_TEXT_LEN + 1),
        });

        assert_eq!(request.validate(), Err(ValidationError::IntentTextTooLong));
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResponseValidationError {
    UnsupportedProtocolVersion { received: u16 },
    ZeroRequestId,
    InvalidStatusResultCombination,
}

impl Response {
    pub fn validate(&self) -> Result<(), ResponseValidationError> {
        if self.protocol_version != PROTOCOL_VERSION {
            return Err(ResponseValidationError::UnsupportedProtocolVersion {
                received: self.protocol_version,
            });
        }

        if self.request_id == 0 {
            return Err(ResponseValidationError::ZeroRequestId);
        }

        let valid = matches!(
            (&self.status, &self.result),
            (RequestStatus::Accepted, None)
                | (RequestStatus::Completed, Some(ActionResult::Ok))
                | (
                    RequestStatus::Completed,
                    Some(ActionResult::MinimizedWindows { .. })
                )
                | (
                    RequestStatus::Completed,
                    Some(ActionResult::Workspaces { .. })
                )
                | (
                    RequestStatus::Completed,
                    Some(ActionResult::OverviewData { .. })
                )
                | (
                    RequestStatus::Rejected,
                    Some(ActionResult::NeedConfirm { .. })
                )
                | (RequestStatus::Rejected, Some(ActionResult::Err { .. }))
                | (RequestStatus::Failed, Some(ActionResult::Err { .. }))
        );

        if !valid {
            return Err(ResponseValidationError::InvalidStatusResultCombination);
        }

        Ok(())
    }
}

#[cfg(test)]
mod response_validation_tests {
    use super::*;

    fn response(status: RequestStatus, result: Option<ActionResult>) -> Response {
        Response {
            protocol_version: PROTOCOL_VERSION,
            request_id: 1,
            status,
            result,
        }
    }

    #[test]
    fn accepts_valid_response_combinations() {
        let cases = [
            response(RequestStatus::Accepted, None),
            response(RequestStatus::Completed, Some(ActionResult::Ok)),
            response(
                RequestStatus::Completed,
                Some(ActionResult::MinimizedWindows {
                    windows: vec![MinimizedWindowInfo {
                        id: 42,
                        title: Some("Terminal".to_owned()),
                        app_id: Some("org.example.Terminal".to_owned()),
                    }],
                }),
            ),
            response(
                RequestStatus::Rejected,
                Some(ActionResult::NeedConfirm {
                    reason: "Confirmation required".to_owned(),
                }),
            ),
            response(
                RequestStatus::Rejected,
                Some(ActionResult::Err {
                    message: "Not permitted".to_owned(),
                }),
            ),
            response(
                RequestStatus::Failed,
                Some(ActionResult::Err {
                    message: "Execution failed".to_owned(),
                }),
            ),
        ];

        for item in cases {
            assert!(item.validate().is_ok());
        }
    }

    #[test]
    fn rejects_completed_response_with_error() {
        let item = response(
            RequestStatus::Completed,
            Some(ActionResult::Err {
                message: "Failed".to_owned(),
            }),
        );
        assert_eq!(
            item.validate(),
            Err(ResponseValidationError::InvalidStatusResultCombination)
        );
    }

    #[test]
    fn rejects_accepted_response_with_result() {
        let item = response(RequestStatus::Accepted, Some(ActionResult::Ok));
        assert_eq!(
            item.validate(),
            Err(ResponseValidationError::InvalidStatusResultCombination)
        );
    }

    #[test]
    fn rejects_failed_response_without_error() {
        let item = response(RequestStatus::Failed, None);
        assert_eq!(
            item.validate(),
            Err(ResponseValidationError::InvalidStatusResultCombination)
        );
    }

    #[test]
    fn rejects_zero_response_id() {
        let mut item = response(RequestStatus::Accepted, None);
        item.request_id = 0;
        assert_eq!(item.validate(), Err(ResponseValidationError::ZeroRequestId));
    }

    #[test]
    fn rejects_unsupported_response_version() {
        let mut item = response(RequestStatus::Accepted, None);
        item.protocol_version = 99;
        assert_eq!(
            item.validate(),
            Err(ResponseValidationError::UnsupportedProtocolVersion { received: 99 })
        );
    }
}
