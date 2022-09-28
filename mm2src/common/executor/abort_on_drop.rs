use futures::future::AbortHandle;

/// The AbortHandle that aborts on drop
pub struct AbortOnDropHandle(Option<AbortHandle>);

impl From<AbortHandle> for AbortOnDropHandle {
    fn from(handle: AbortHandle) -> Self { AbortOnDropHandle(Some(handle)) }
}

impl AbortOnDropHandle {
    pub fn into_handle(mut self) -> AbortHandle { self.0.take().expect("`AbortHandle` Must be initialized") }
}

impl Drop for AbortOnDropHandle {
    #[inline(always)]
    fn drop(&mut self) {
        if let Some(handle) = self.0.take() {
            handle.abort();
        }
    }
}
