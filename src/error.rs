use std::{
    fmt::{Debug, Formatter},
    panic::Location,
};

use anyhow::anyhow;
use derivative::Derivative;

fn to_string_fmt<T: ToString>(t: &T, f: &mut Formatter<'_>) -> Result<(), std::fmt::Error> {
    f.write_str(&t.to_string())
}

#[derive(Derivative)]
#[derivative(Debug)]
pub(crate) struct TraceError {
    #[derivative(Debug(format_with = "self::to_string_fmt"))]
    source: &'static Location<'static>,
    #[derivative(Debug(format_with = "self::to_string_fmt"))]
    error: String,
    message: Option<String>,
}

impl TraceError {
    fn with_context(mut self, message: String) -> Self {
        self.message = Some(message);
        self
    }
}

pub(crate) type Trace<T> = Result<T, TraceError>;

pub(crate) trait TraceExt {
    type Output;
    fn trace(self) -> Result<Self::Output, TraceError>;
}

impl<T> TraceExt for Option<T> {
    type Output = T;
    #[track_caller]
    fn trace(self) -> Result<Self::Output, TraceError> {
        match self {
            Some(x) => Ok(x),
            None => Err(TraceError::from(anyhow!("NoneError"))),
        }
    }
}

pub(crate) trait ContextExt {
    type Output;
    fn with_context(self, message: impl Into<String>) -> TraceError;
}

impl<T: Not + Debug> ContextExt for T {
    type Output = T;
    #[track_caller]
    fn with_context(self, message: impl Into<String>) -> TraceError {
        let message = message.into();
        TraceError::from(self).with_context(message)
    }
}

auto trait Not {}
impl !Not for TraceError {}

impl<T: Not + Debug> From<T> for TraceError {
    #[track_caller]
    fn from(error: T) -> Self {
        TraceError {
            source: std::intrinsics::caller_location(),
            error: format!("{error:#?}"),
            message: None,
        }
    }
}

#[test]
fn trace_error() {
    #![allow(non_upper_case_globals)]

    use anyhow::anyhow;

    const creative_error_msg: &str = "enhance your calm";

    const line: u32 = line!() + 2;
    fn inner() -> Result<(), TraceError> {
        Err(anyhow!(creative_error_msg))?
    }
    let col = {
        {};
        column!()
    };

    let err = inner().unwrap_err();

    assert_eq!(
        format!("{err:?}"),
        format!(
            "TraceError {{ source: {}:{}:{}, error: {:?}, message: None }}",
            file!(),
            line,
            col,
            creative_error_msg,
        )
    );
}
