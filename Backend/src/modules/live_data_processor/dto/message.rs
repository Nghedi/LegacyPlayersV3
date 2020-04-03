use crate::modules::live_data_processor::dto::MessageType;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Message {
  pub api_version: u8,
  pub message_length: u8,
  pub timestamp: u64,
  pub message_type: MessageType
}