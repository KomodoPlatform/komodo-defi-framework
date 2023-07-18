mod activation_scheme_impl;
mod init_activation_scheme;

pub(super) use activation_scheme_impl::get_activation_scheme;
#[cfg(test)]
pub(super) use init_activation_scheme::get_activation_scheme_path;
pub(super) use init_activation_scheme::init_activation_scheme;
