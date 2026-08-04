// ── Working memory ─────────────────────────────────────────
// Layer 1: Ephemeral in-RAM storage for current conversation.
// No persistence. Cleared on session end.

use std::collections::VecDeque;

#[derive(Clone)]
pub struct Message {
    pub role: String,
    pub content: String,
}

pub struct WorkingMemory {
    messages: VecDeque<Message>,
    max_messages: usize,
    pub session_id: String,
}

#[allow(dead_code)]
impl WorkingMemory {
    pub fn new(session_id: &str) -> Self {
        Self {
            messages: VecDeque::new(),
            max_messages: 50,
            session_id: session_id.to_string(),
        }
    }

    pub fn add_message(&mut self, role: &str, content: &str) {
        self.messages.push_back(Message {
            role: role.to_string(),
            content: content.to_string(),
        });
        while self.messages.len() > self.max_messages {
            self.messages.pop_front();
        }
    }

    pub fn get_context_window(&self) -> Vec<&Message> {
        self.messages.iter().collect()
    }

    pub fn clear(&mut self) {
        self.messages.clear();
    }
}
