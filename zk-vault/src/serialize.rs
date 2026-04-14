use zk_domain::note::NoteFrontmatter;

#[must_use]
pub fn serialize_frontmatter(frontmatter: &NoteFrontmatter) -> String {
    let mut output = String::new();

    push_yaml_scalar_line(&mut output, "id", &frontmatter.id.to_string());
    push_yaml_scalar_line(&mut output, "title", &frontmatter.title);
    push_yaml_string_list(&mut output, "tags", &frontmatter.tags);

    if let Some(note_type) = &frontmatter.note_type {
        push_yaml_scalar_line(&mut output, "type", note_type);
    }
    if !frontmatter.aliases.is_empty() {
        push_yaml_string_list(&mut output, "aliases", &frontmatter.aliases);
    }
    if let Some(created) = &frontmatter.created {
        push_yaml_scalar_line(&mut output, "created", created);
    }
    if let Some(updated) = &frontmatter.updated {
        push_yaml_scalar_line(&mut output, "updated", updated);
    }
    if let Some(source) = &frontmatter.source {
        push_yaml_scalar_line(&mut output, "source", source);
    }
    if let Some(status) = &frontmatter.status {
        push_yaml_scalar_line(&mut output, "status", status);
    }
    if !frontmatter.sources.is_empty() {
        let source_values: Vec<String> = frontmatter
            .sources
            .iter()
            .map(ToString::to_string)
            .collect();
        push_yaml_string_list(&mut output, "sources", &source_values);
    }

    for (key, value) in &frontmatter.extra {
        push_yaml_extra_value(&mut output, key, value);
    }

    output
}

#[must_use]
pub fn render_note_file(frontmatter: &NoteFrontmatter, body: &str) -> String {
    let serialized_frontmatter = serialize_frontmatter(frontmatter);
    let mut output = String::new();
    output.push_str("---\n");
    output.push_str(&serialized_frontmatter);
    output.push_str("---\n");
    output.push_str(body);

    if !body.is_empty() && !body.ends_with('\n') {
        output.push('\n');
    }

    output
}

fn push_yaml_scalar_line(output: &mut String, key: &str, value: &str) {
    output.push_str(key);
    output.push_str(": ");
    output.push_str(value);
    output.push('\n');
}

fn push_yaml_string_list(output: &mut String, key: &str, values: &[String]) {
    output.push_str(key);
    output.push_str(":\n");
    for value in values {
        output.push_str("  - ");
        output.push_str(value);
        output.push('\n');
    }
}

fn push_yaml_extra_value(output: &mut String, key: &str, value: &serde_yaml::Value) {
    let serialized_value = sanitize_serde_yaml_value(value);
    if !serialized_value.contains('\n') {
        output.push_str(key);
        output.push_str(": ");
        output.push_str(&serialized_value);
        output.push('\n');
        return;
    }

    output.push_str(key);
    output.push_str(":\n");
    for line in serialized_value.lines() {
        output.push_str("  ");
        output.push_str(line);
        output.push('\n');
    }
}

fn sanitize_serde_yaml_value(value: &serde_yaml::Value) -> String {
    let serialized = serde_yaml::to_string(value).expect("extra value should serialize");
    let without_document_start = serialized
        .strip_prefix("---\n")
        .or_else(|| serialized.strip_prefix("---"))
        .unwrap_or(&serialized);

    without_document_start.trim_end().to_string()
}
