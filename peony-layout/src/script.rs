#[derive(Debug, Clone, Default)]
pub struct ScriptLayout {
    pub output_sections: Vec<ScriptOutputSection>,
    pub keep_patterns: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct ScriptOutputSection {
    pub name: String,
    pub patterns: Vec<String>,
}

impl ScriptLayout {
    pub fn output_for_input(&self, input_name: &[u8]) -> Option<&str> {
        let name = std::str::from_utf8(input_name).ok()?;
        self.output_sections
            .iter()
            .find(|out| {
                out.patterns
                    .iter()
                    .any(|pat| script_pattern_matches(pat, name))
            })
            .map(|out| out.name.as_str())
    }

    pub fn order_of(&self, name: &str) -> Option<usize> {
        self.output_sections.iter().position(|out| out.name == name)
    }

    pub fn keeps_input(&self, input_name: &[u8]) -> bool {
        let Ok(name) = std::str::from_utf8(input_name) else {
            return false;
        };
        self.keep_patterns
            .iter()
            .any(|pat| script_pattern_matches(pat, name))
    }
}

fn script_pattern_matches(pattern: &str, name: &str) -> bool {
    if pattern == "*" {
        return true;
    }
    let mut rest = name;
    let mut parts = pattern.split('*').peekable();
    let anchored_start = !pattern.starts_with('*');
    let anchored_end = !pattern.ends_with('*');

    if let Some(first) = parts.next()
        && !first.is_empty()
    {
        if anchored_start {
            let Some(stripped) = rest.strip_prefix(first) else {
                return false;
            };
            rest = stripped;
        } else if let Some(pos) = rest.find(first) {
            rest = &rest[pos + first.len()..];
        } else {
            return false;
        }
    }

    let mut last_non_empty = "";
    for part in parts {
        if part.is_empty() {
            continue;
        }
        last_non_empty = part;
        let Some(pos) = rest.find(part) else {
            return false;
        };
        rest = &rest[pos + part.len()..];
    }

    !anchored_end || pattern.ends_with(last_non_empty) && rest.is_empty()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn script_patterns_match_common_linker_wildcards() {
        assert!(script_pattern_matches(".text*", ".text.startup"));
        assert!(script_pattern_matches("*.init_array", ".rela.init_array"));
        assert!(script_pattern_matches(".rodata", ".rodata"));
        assert!(!script_pattern_matches(".data", ".data.rel.ro"));

        let script = ScriptLayout {
            output_sections: vec![ScriptOutputSection {
                name: ".fast".to_string(),
                patterns: vec![".text.hot*".to_string(), ".init".to_string()],
            }],
            keep_patterns: vec![".init_array*".to_string()],
        };
        assert_eq!(script.output_for_input(b".text.hot.foo"), Some(".fast"));
        assert!(script.keeps_input(b".init_array.1"));
        assert!(!script.keeps_input(b".fini_array.1"));
    }
}
