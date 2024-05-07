#[macro_export]
macro_rules! write_confirmation_settings {
    ($writer:expr, $host:ident, $width:ident) => {
        if $host.conf_settings.is_some() {
            let output = format_confirmation_settings($host.conf_settings.as_ref().unwrap());
            writeln_field($writer, concat!(stringify!($host), ".conf_settings"), output, $width)
        }
    };
}

#[macro_export]
macro_rules! write_base_rel {
    ($writer:ident, $host:expr, $width:ident) => {
        writeln_field(
            $writer,
            concat!(stringify!($host), ".(base,rel)"),
            format!(
                "{}({}), {}({})",
                $host.base,
                format_ratio(&$host.base_amount, COMMON_PRECISION)
                    .map_err(|error| error_anyhow!("Failed to format base_amount: {error}"))?,
                $host.rel,
                format_ratio(&$host.rel_amount, COMMON_PRECISION)
                    .map_err(|error| error_anyhow!("Failed to format rel_amount: {error}"))?,
            ),
            $width,
        );
    };
}

#[macro_export]
macro_rules! write_connected {
    ($writer:ident, $connected:expr, $width:ident) => {
        writeln_field(
            $writer,
            concat!(stringify!($connected), ".(taker,maker)"),
            format!("{},{}", $connected.taker_order_uuid, $connected.maker_order_uuid),
            $width,
        );
        writeln_field(
            $writer,
            concat!(stringify!($connected), ".(sender, dest)"),
            format!("{},{}", $connected.sender_pubkey, $connected.dest_pub_key),
            $width,
        );
    };
}

pub(super) use {write_base_rel, write_confirmation_settings, write_connected};
