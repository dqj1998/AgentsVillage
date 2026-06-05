use super::{AppRequest, Intent, RequestPayload};

/// Compiles a platform-agnostic AppRequest into an Intent
pub struct IntentCompiler;

impl IntentCompiler {
    pub fn new() -> Self {
        Self
    }

    pub fn compile(&self, request: &AppRequest) -> Intent {
        match &request.payload {
            RequestPayload::Command { name, args } => {
                if name == "new" {
                    Intent::ResetSession
                } else {
                    Intent::Command {
                        name: name.clone(),
                        args: args.clone(),
                    }
                }
            }
            RequestPayload::Message(text) => Intent::Chat {
                user_text: text.clone(),
                author: request.platform_user.clone(),
            },
        }
    }
}

impl Default for IntentCompiler {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{AppRequest, RequestPayload};

    fn make_request(payload: RequestPayload) -> AppRequest {
        AppRequest {
            agent_id: "discord-1-2".to_string(),
            platform_user: "testuser".to_string(),
            timestamp: "2024-01-01 00:00:00 UTC".to_string(),
            payload,
        }
    }

    #[test]
    fn message_compiles_to_chat_intent() {
        let compiler = IntentCompiler::new();
        let req = make_request(RequestPayload::Message("hello world".to_string()));
        let intent = compiler.compile(&req);
        assert!(matches!(intent, Intent::Chat { .. }));
        if let Intent::Chat { user_text, author } = intent {
            assert_eq!(user_text, "hello world");
            assert_eq!(author, "testuser");
        }
    }

    #[test]
    fn new_command_compiles_to_reset_session() {
        let compiler = IntentCompiler::new();
        let req = make_request(RequestPayload::Command {
            name: "new".to_string(),
            args: vec![],
        });
        let intent = compiler.compile(&req);
        assert!(matches!(intent, Intent::ResetSession));
    }

    #[test]
    fn other_command_compiles_to_command_intent() {
        let compiler = IntentCompiler::new();
        let req = make_request(RequestPayload::Command {
            name: "help".to_string(),
            args: vec!["topic".to_string()],
        });
        let intent = compiler.compile(&req);
        assert!(matches!(intent, Intent::Command { .. }));
    }
}
