/// Embedded default templates using compile-time inclusion
///
/// These templates are bundled into the binary and serve as fallbacks
/// when custom templates are not available.

/// Daily standup template for engineering/product teams
pub const DAILY_STANDUP: &str = include_str!("../../../templates/daily_standup.json");

/// Standard meeting notes template
pub const STANDARD_MEETING: &str = include_str!("../../../templates/standard_meeting.json");

/// Chinese official-document style minutes (纪要)
pub const GOVERNMENT_MINUTES: &str = include_str!("../../../templates/government_minutes.json");

/// The four meeting kinds carried over from the earlier minutes workbench
pub const PARTY_GROUP: &str = include_str!("../../../templates/party_group.json");
pub const SPECIAL_TOPIC: &str = include_str!("../../../templates/special_topic.json");
pub const COORDINATION: &str = include_str!("../../../templates/coordination.json");
pub const REGULAR_MEETING: &str = include_str!("../../../templates/regular_meeting.json");

/// Registry of all built-in templates
///
/// Maps template identifiers to their embedded JSON content
pub fn get_builtin_templates() -> Vec<(&'static str, &'static str)> {
    vec![
        ("daily_standup", DAILY_STANDUP),
        ("standard_meeting", STANDARD_MEETING),
        ("government_minutes", GOVERNMENT_MINUTES),
        ("party_group", PARTY_GROUP),
        ("special_topic", SPECIAL_TOPIC),
        ("coordination", COORDINATION),
        ("regular_meeting", REGULAR_MEETING),
    ]
}

/// Get a built-in template by identifier
///
/// # Arguments
/// * `id` - Template identifier (e.g., "daily_standup", "standard_meeting")
///
/// # Returns
/// The template JSON content if found, None otherwise
pub fn get_builtin_template(id: &str) -> Option<&'static str> {
    match id {
        "daily_standup" => Some(DAILY_STANDUP),
        "standard_meeting" => Some(STANDARD_MEETING),
        "government_minutes" => Some(GOVERNMENT_MINUTES),
        "party_group" => Some(PARTY_GROUP),
        "special_topic" => Some(SPECIAL_TOPIC),
        "coordination" => Some(COORDINATION),
        "regular_meeting" => Some(REGULAR_MEETING),
        _ => None,
    }
}

/// List all built-in template identifiers
pub fn list_builtin_template_ids() -> Vec<&'static str> {
    vec![
        "daily_standup",
        "standard_meeting",
        "government_minutes",
        "party_group",
        "special_topic",
        "coordination",
        "regular_meeting",
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_builtin_templates_valid_json() {
        for (id, content) in get_builtin_templates() {
            let result = serde_json::from_str::<serde_json::Value>(content);
            assert!(
                result.is_ok(),
                "Built-in template '{}' contains invalid JSON: {:?}",
                id,
                result.err()
            );
        }
    }

    #[test]
    fn test_get_builtin_template() {
        assert!(get_builtin_template("daily_standup").is_some());
        assert!(get_builtin_template("standard_meeting").is_some());
        assert!(get_builtin_template("government_minutes").is_some());
        assert!(get_builtin_template("party_group").is_some());
        assert!(get_builtin_template("regular_meeting").is_some());
        assert!(get_builtin_template("nonexistent").is_none());
    }
}
