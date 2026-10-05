# main.rs

Top-level clap dispatcher for memory and chat. Each crate owns its CLI and returns an exit code or error. Model/default flags live in ai-chat, avoiding protocol logic in the binary.
