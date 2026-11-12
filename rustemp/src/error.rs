use shared::error::write_wire_diagnostic;
use wllib::error::WireError;
use wllib::wire::MessageError;

pub enum AppError {
  Wire(WireError),
}

impl AppError {
  pub fn write_diagnostic(&self) {
    match self {
      AppError::Wire(e) => write_wire_diagnostic("[rustemp]", e),
    }
  }
}

impl From<WireError> for AppError {
  fn from(e: WireError) -> Self {
    Self::Wire(e)
  }
}
impl From<MessageError> for AppError {
  fn from(e: MessageError) -> Self {
    Self::Wire(WireError::Parse(e))
  }
}
