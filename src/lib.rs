// Avoid clippy warning for format! function
#[allow(clippy::cast_possible_wrap, clippy::uninlined_format_args)]
pub mod chat_server;

#[cfg(test)]
mod tests {
    mod ack_nack_tests;
    mod flooding_tests;
    mod message_tests;
    mod sc_commands_tests;
}
