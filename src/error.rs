#[derive(Debug, Clone, Responder)]
#[response(status = 400)]
pub struct ExitNodeError {
    pub description: String,
}

pub struct EntryNodeError {
    pub description: String,
}

impl ExitNodeError {
    pub fn f(from_string: String) -> Self {
        Self {
            description: from_string,
        }
    }
    pub fn closed() -> Self {
        Self {
            description: String::from("The connection is already closed"),
        }
    }
}

impl EntryNodeError {
    pub fn f(from_string: String) -> Self {
        Self {
            description: from_string,
        }
    }
}

// Conversions for ExitNodeError
impl From<std::io::Error> for ExitNodeError {
    fn from(e: std::io::Error) -> Self {
        Self::f(format!("{:?}", e))
    }
}

impl From<base64::DecodeError> for ExitNodeError {
    fn from(e: base64::DecodeError) -> Self {
        Self::f(format!("{:?}", e))
    }
}

// Conversions for EntryNodeError
impl From<hyper::Error> for EntryNodeError {
    fn from(e: hyper::Error) -> Self {
        Self::f(format!("{:?}", e))
    }
}

impl From<std::num::ParseIntError> for EntryNodeError {
    fn from(e: std::num::ParseIntError) -> Self {
        Self::f(format!("{:?}", e))
    }
}

// Maybe shared utils should not raise that kind of error?
impl From<ExitNodeError> for EntryNodeError {
    fn from(e: ExitNodeError) -> Self {
        Self {
            description: e.description,
        }
    }
}
