#![feature(negative_impls)]
#![feature(auto_traits)]

pub mod map_mm_error;
pub mod map_to_mm;
pub mod map_to_mm_fut;
pub mod mm_error;
pub mod mm_json_error;
pub mod or_mm_error;

pub mod prelude {
    pub use crate::map_mm_error::MapMmError;
    pub use crate::map_to_mm::MapToMmResult;
    pub use crate::map_to_mm_fut::MapToMmFutureExt;
    pub use crate::mm_error::{MmError, MmResult, NotEqual, NotMmError, SerMmErrorType};
    pub use crate::mm_json_error::MmJsonError;
    pub use crate::or_mm_error::OrMmError;
    pub use ser_error::SerializeErrorType;
}
