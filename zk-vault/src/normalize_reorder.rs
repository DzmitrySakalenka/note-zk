pub(crate) fn requires_field_reorder(raw_frontmatter_yaml: &str) -> bool {
    let observed = collect_top_level_yaml_keys(raw_frontmatter_yaml);
    if observed.len() < 2 {
        return false;
    }

    let mut expected = observed.clone();
    expected.sort_by(|left, right| {
        let left_rank = canonical_key_rank(left);
        let right_rank = canonical_key_rank(right);
        left_rank.cmp(&right_rank).then_with(|| left.cmp(right))
    });

    observed != expected
}

fn collect_top_level_yaml_keys(raw_frontmatter_yaml: &str) -> Vec<String> {
    raw_frontmatter_yaml
        .lines()
        .filter_map(extract_top_level_key)
        .collect()
}

fn extract_top_level_key(line: &str) -> Option<String> {
    let indent = line.chars().take_while(|ch| *ch == ' ').count();
    if indent != 0 {
        return None;
    }

    let trimmed = line.trim();
    if trimmed.is_empty() || trimmed.starts_with('#') || trimmed.starts_with('-') {
        return None;
    }

    let colon_index = trimmed.find(':')?;
    let key = trimmed[..colon_index].trim();
    if key.is_empty() {
        return None;
    }

    Some(strip_yaml_quotes(key))
}

fn strip_yaml_quotes(key: &str) -> String {
    if key.len() >= 2
        && ((key.starts_with('"') && key.ends_with('"'))
            || (key.starts_with('\'') && key.ends_with('\'')))
    {
        return key[1..key.len() - 1].to_string();
    }
    key.to_string()
}

fn canonical_key_rank(key: &str) -> usize {
    match key {
        "id" => 0,
        "title" => 1,
        "tags" => 2,
        "type" => 3,
        "aliases" => 4,
        "created" => 5,
        "updated" => 6,
        "source" => 7,
        "status" => 8,
        "sources" => 9,
        _ => 10,
    }
}
