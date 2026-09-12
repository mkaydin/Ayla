use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum NodeType {
    Person,
    Project,
    Tool,
    Concept,
    Document,
    Other,
}
