//! The skill that tells an agent how to use Sliderino: `SKILL.md`, served
//! as an MCP resource and printed by `sliderino skill`. The book includes
//! the text between its `guide` anchors in `docs/agent-api.md`.

/// The skill, with its frontmatter.
pub const SKILL: &str = include_str!("SKILL.md");

/// The URI of the skill as an MCP resource.
pub const URI: &str = "skill://sliderino/SKILL.md";

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::tools;

    #[test]
    fn the_skill_has_a_frontmatter_and_the_guide_anchors() {
        assert!(SKILL.starts_with("---\nname: sliderino\ndescription: "));
        let open = SKILL.find("<!-- ANCHOR: guide -->").unwrap();
        let close = SKILL.find("<!-- ANCHOR_END: guide -->").unwrap();
        assert!(open < close);
    }

    #[test]
    fn the_skill_names_every_tool() {
        for spec in tools::specs() {
            assert!(
                SKILL.contains(&format!("`{}`", spec.name)),
                "SKILL.md does not name the tool {}",
                spec.name
            );
        }
    }

    /// The `op` tags of `Op`, read from its source.
    fn op_names() -> Vec<String> {
        let source = include_str!("ops.rs");
        let body = source.split("pub enum Op {").nth(1).unwrap();
        let body = &body[..body.find("\n}").unwrap()];
        body.lines()
            .filter_map(|line| line.strip_prefix("    "))
            .filter(|line| line.starts_with(|c: char| c.is_ascii_uppercase()))
            .map(|line| {
                let name = line.split([' ', '{', '(', ',']).next().unwrap();
                let mut snake = String::new();
                for (index, c) in name.chars().enumerate() {
                    if c.is_ascii_uppercase() && index > 0 {
                        snake.push('_');
                    }
                    snake.push(c.to_ascii_lowercase());
                }
                snake
            })
            .collect()
    }

    #[test]
    fn the_skill_names_every_operation() {
        let names = op_names();
        assert!(names.contains(&"add_slide".to_string()));
        assert!(names.contains(&"set_table_sizing".to_string()));
        for name in names {
            assert!(
                SKILL.contains(&format!("`{name}`")),
                "SKILL.md does not name the operation {name}"
            );
        }
    }
}
