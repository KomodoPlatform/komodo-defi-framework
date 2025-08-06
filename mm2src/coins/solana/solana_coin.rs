use std::ops::Deref;
use std::sync::Arc;

#[derive(Clone)]
pub struct SolanaCoin(Arc<SolanaCoinFields>);

pub struct SolanaCoinFields {}

impl Deref for SolanaCoin {
    type Target = SolanaCoinFields;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
