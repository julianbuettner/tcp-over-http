#[derive(Debug)]
struct TraceError {}

auto trait Not {}
impl !Not for TraceError {}

impl<T: Not> From<T> for TraceError {
    #[track_caller]
    fn from(_: T) -> Self {
        //todo actually use this
        TraceError {}
    }
}

#[test]
fn trace_error() {
    use anyhow::anyhow;
    fn inner() -> Result<(), TraceError> {
        Err(anyhow!("penis"))?
    }
    inner().unwrap_err();
}
