#[derive(Debug, Clone, PartialEq)]
pub enum WorkflowStatus {
    Completed,
    Cancelled,
    Faulted(String),
}

pub fn parse_docstring(docstring: &str) -> String {
    docstring
        .lines()
        .skip_while(|line| line.trim() == "yaml" || line.trim() == "json")
        .collect::<Vec<_>>()
        .join("\n")
}
