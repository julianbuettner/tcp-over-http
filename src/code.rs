use super::error::ExitNodeError;

pub fn decode(s: &str) -> Result<Vec<u8>, ExitNodeError> {
    Ok(base64::decode(s)?)
}

pub fn encode(d: Vec<u8>) -> String {
    base64::encode(d)
}
