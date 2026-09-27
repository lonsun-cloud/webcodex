use serde::{Deserialize, Serialize};

pub const MAX_SKILL_NAME_CHARS: usize = 96;
pub const MAX_SKILL_DESCRIPTION_CHARS: usize = 1024;
pub const MAX_SKILL_DEFINITION_BYTES: usize = 64 * 1024;
pub const MAX_SKILL_FRONTMATTER_LINES: usize = 64;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkillMetadata {
    pub name: String,
    pub description: String,
}

#[derive(Debug, Deserialize)]
struct SkillFrontmatter {
    name: Option<String>,
    description: Option<String>,
}

/// Parse the canonical bounded Agent Skill frontmatter used by both Control
/// project discovery and the Runner operator-store installer. Only explicit
/// scalar `name` and `description` metadata are accepted; bodies are never
/// searched for inferred metadata.
pub fn parse_skill_metadata(text: &str) -> Result<SkillMetadata, &'static str> {
    if text.len() > MAX_SKILL_DEFINITION_BYTES {
        return Err("skill_definition_too_large");
    }
    let frontmatter = extract_frontmatter(text)?;
    let parsed = match serde_norway::from_str::<SkillFrontmatter>(&frontmatter) {
        Ok(parsed) => parsed,
        Err(_) => parse_legacy_frontmatter(&frontmatter)?,
    };
    let name = parsed.name.ok_or("skill_name_missing")?;
    let description = parsed.description.ok_or("skill_description_missing")?;
    if name.is_empty()
        || name.chars().count() > MAX_SKILL_NAME_CHARS
        || name.chars().any(char::is_control)
    {
        return Err("skill_name_invalid");
    }
    if description.is_empty()
        || description.chars().count() > MAX_SKILL_DESCRIPTION_CHARS
        || description
            .chars()
            .any(|ch| ch.is_control() && !matches!(ch, '\n' | '\r' | '\t'))
    {
        return Err("skill_description_invalid");
    }
    Ok(SkillMetadata { name, description })
}

fn extract_frontmatter(text: &str) -> Result<String, &'static str> {
    let mut lines = text.lines();
    let Some(first) = lines.next() else {
        return Err("skill_frontmatter_missing");
    };
    if first.trim_start_matches('\u{feff}').trim() != "---" {
        return Err("skill_frontmatter_missing");
    }
    let mut frontmatter = String::new();
    for line in lines.take(MAX_SKILL_FRONTMATTER_LINES) {
        if line.trim_end() == "---" {
            return Ok(frontmatter);
        }
        frontmatter.push_str(line);
        frontmatter.push('\n');
    }
    Err("skill_frontmatter_unclosed")
}

fn parse_legacy_frontmatter(frontmatter: &str) -> Result<SkillFrontmatter, &'static str> {
    let mut name = None::<String>;
    let mut description = None::<String>;
    for line in frontmatter.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let Some((key, raw_value)) = trimmed.split_once(':') else {
            continue;
        };
        let key = key.trim();
        if !matches!(key, "name" | "description") {
            continue;
        }
        let value = parse_frontmatter_scalar(raw_value.trim())?;
        match key {
            "name" if name.is_none() => name = Some(value),
            "description" if description.is_none() => description = Some(value),
            _ => return Err("skill_frontmatter_duplicate_field"),
        }
    }
    Ok(SkillFrontmatter { name, description })
}

fn parse_frontmatter_scalar(raw: &str) -> Result<String, &'static str> {
    if raw.is_empty()
        || matches!(
            raw.as_bytes().first(),
            Some(b'|' | b'>' | b'[' | b'{' | b'&' | b'*' | b'!')
        )
    {
        return Err("skill_frontmatter_scalar_invalid");
    }
    let value = if raw.len() >= 2 && raw.starts_with('"') && raw.ends_with('"') {
        let inner = &raw[1..raw.len() - 1];
        if inner.contains('"') || inner.contains('\\') {
            return Err("skill_frontmatter_scalar_invalid");
        }
        inner
    } else if raw.len() >= 2 && raw.starts_with('\'') && raw.ends_with('\'') {
        let inner = &raw[1..raw.len() - 1];
        if inner.contains('\'') {
            return Err("skill_frontmatter_scalar_invalid");
        }
        inner
    } else {
        raw.split(" #").next().unwrap_or(raw).trim()
    };
    Ok(value.trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounded_parser_requires_explicit_metadata() {
        let parsed = parse_skill_metadata(
            "---\nname: demo\ndescription: 'Use demo safely'\nlicense: MIT\n---\nPRIVATE_BODY",
        )
        .unwrap();
        assert_eq!(parsed.name, "demo");
        assert_eq!(parsed.description, "Use demo safely");
        for invalid in [
            "# no frontmatter\nname: guessed",
            "---\ndescription: only desc\n---\nname in body",
            "---\nname: x\n---\nbody description",
            "---\nname: x\ndescription: [block]\n---",
            "---\nname: x\ndescription: {text: block}\n---",
        ] {
            assert!(parse_skill_metadata(invalid).is_err(), "{invalid}");
        }
    }

    #[test]
    fn parser_preserves_legacy_duplicate_field_error() {
        assert_eq!(
            parse_skill_metadata(
                "---\nname: demo\nname: duplicate\ndescription: duplicate names are invalid\n---",
            ),
            Err("skill_frontmatter_duplicate_field")
        );
    }

    #[test]
    fn parser_accepts_yaml_block_string_scalars() {
        let folded = parse_skill_metadata(
            "---\nname: demo\ndescription: >\n  Build useful tools with\n  reusable primitives.\n---",
        )
        .unwrap();
        assert_eq!(
            folded.description,
            "Build useful tools with reusable primitives.\n"
        );

        let folded_stripped = parse_skill_metadata(
            "---\nname: demo\ndescription: >-\n  Build useful tools with\n  reusable primitives.\n---",
        )
        .unwrap();
        assert_eq!(
            folded_stripped.description,
            "Build useful tools with reusable primitives."
        );

        let literal = parse_skill_metadata(
            "---\nname: demo\ndescription: |\n  First line.\n  Second line.\n---",
        )
        .unwrap();
        assert_eq!(literal.description, "First line.\nSecond line.\n");
    }

    #[test]
    fn parser_keeps_indented_document_marker_inside_block_scalar() {
        let parsed = parse_skill_metadata(
            "---\nname: demo\ndescription: |\n  First line.\n  ---\n  Last line.\n---",
        )
        .unwrap();
        assert_eq!(parsed.description, "First line.\n---\nLast line.\n");
    }

    #[test]
    fn parser_accepts_spec_length_block_description() {
        let description = "x".repeat(661);
        let skill = format!("---\nname: demo\ndescription: >-\n  {description}\n---");
        let parsed = parse_skill_metadata(&skill).unwrap();
        assert_eq!(parsed.description, description);

        let too_long = "x".repeat(MAX_SKILL_DESCRIPTION_CHARS + 1);
        let skill = format!("---\nname: demo\ndescription: >-\n  {too_long}\n---");
        assert_eq!(
            parse_skill_metadata(&skill),
            Err("skill_description_invalid")
        );
    }

    #[test]
    fn parser_accepts_crlf_and_utf8_bom() {
        let parsed = parse_skill_metadata(
            "\u{feff}---\r\nname: demo\r\ndescription: >-\r\n  Works on Windows too.\r\n---\r\n",
        )
        .unwrap();
        assert_eq!(parsed.name, "demo");
        assert_eq!(parsed.description, "Works on Windows too.");
    }
}
