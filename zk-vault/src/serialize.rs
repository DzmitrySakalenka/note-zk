use zk_domain::note::NoteFrontmatter;

/// Serializes canonical note frontmatter into the workspace YAML form.
///
/// # Errors
///
/// Returns [`serde_yaml::Error`] if any frontmatter value cannot be serialized
/// into YAML.
pub fn serialize_frontmatter(frontmatter: &NoteFrontmatter) -> Result<String, serde_yaml::Error> {
    let mut output = String::new();

    push_yaml_scalar_line(&mut output, "id", &frontmatter.id.to_string())?;
    push_yaml_scalar_line(&mut output, "title", &frontmatter.title)?;
    push_yaml_string_list(&mut output, "tags", &frontmatter.tags)?;

    if let Some(note_type) = &frontmatter.note_type {
        push_yaml_scalar_line(&mut output, "type", note_type)?;
    }
    if !frontmatter.aliases.is_empty() {
        push_yaml_string_list(&mut output, "aliases", &frontmatter.aliases)?;
    }
    if let Some(created) = &frontmatter.created {
        push_yaml_scalar_line(&mut output, "created", created)?;
    }
    if let Some(updated) = &frontmatter.updated {
        push_yaml_scalar_line(&mut output, "updated", updated)?;
    }
    if let Some(source) = &frontmatter.source {
        push_yaml_scalar_line(&mut output, "source", source)?;
    }
    if let Some(status) = &frontmatter.status {
        push_yaml_scalar_line(&mut output, "status", status)?;
    }
    if !frontmatter.sources.is_empty() {
        let source_values: Vec<String> = frontmatter
            .sources
            .iter()
            .map(ToString::to_string)
            .collect();
        push_yaml_string_list(&mut output, "sources", &source_values)?;
    }

    for (key, value) in &frontmatter.extra {
        push_yaml_extra_value(&mut output, key, value)?;
    }

    Ok(output)
}

/// Renders a full note file with YAML frontmatter and markdown body.
///
/// # Errors
///
/// Returns [`serde_yaml::Error`] if the frontmatter cannot be serialized into
/// YAML.
pub fn render_note_file(
    frontmatter: &NoteFrontmatter,
    body: &str,
) -> Result<String, serde_yaml::Error> {
    let serialized_frontmatter = serialize_frontmatter(frontmatter)?;
    let mut output = String::new();
    output.push_str("---\n");
    output.push_str(&serialized_frontmatter);
    output.push_str("---\n");
    output.push_str(body);

    if !body.is_empty() && !body.ends_with('\n') {
        output.push('\n');
    }

    Ok(output)
}

fn push_yaml_scalar_line(
    output: &mut String,
    key: &str,
    value: &str,
) -> Result<(), serde_yaml::Error> {
    let serialized_value = sanitize_yaml_string_scalar(value)?;
    push_yaml_mapping_value(output, key, &serialized_value);
    Ok(())
}

fn push_yaml_string_list(
    output: &mut String,
    key: &str,
    values: &[String],
) -> Result<(), serde_yaml::Error> {
    output.push_str(key);
    output.push_str(":\n");
    for value in values {
        let serialized_value = sanitize_yaml_string_scalar(value)?;
        push_yaml_list_item(output, &serialized_value);
    }
    Ok(())
}

fn push_yaml_extra_value(
    output: &mut String,
    key: &str,
    value: &serde_yaml::Value,
) -> Result<(), serde_yaml::Error> {
    let serialized_value = sanitize_serde_yaml_value(value)?;
    push_yaml_mapping_value(output, key, &serialized_value);
    Ok(())
}

fn push_yaml_mapping_value(output: &mut String, key: &str, serialized_value: &str) {
    if !serialized_value.contains('\n') {
        output.push_str(key);
        output.push_str(": ");
        output.push_str(serialized_value);
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

fn push_yaml_list_item(output: &mut String, serialized_value: &str) {
    let mut lines = serialized_value.lines();
    let first_line = lines.next().unwrap_or_default();
    output.push_str("  - ");
    output.push_str(first_line);
    output.push('\n');

    let continuation_lines: Vec<&str> = lines.collect();
    if continuation_lines.is_empty() {
        return;
    }

    let continuation_indent = minimum_nonempty_line_indent(&continuation_lines);
    for line in continuation_lines {
        output.push_str("    ");
        output.push_str(strip_leading_spaces(line, continuation_indent));
        output.push('\n');
    }
}

fn minimum_nonempty_line_indent(lines: &[&str]) -> usize {
    lines
        .iter()
        .filter(|line| !line.is_empty())
        .map(|line| line.bytes().take_while(|byte| *byte == b' ').count())
        .min()
        .unwrap_or(0)
}

fn strip_leading_spaces(line: &str, spaces_to_strip: usize) -> &str {
    let available_prefix_spaces = line.bytes().take_while(|byte| *byte == b' ').count();
    let prefix_len = spaces_to_strip.min(available_prefix_spaces);
    &line[prefix_len..]
}

fn sanitize_yaml_string_scalar(value: &str) -> Result<String, serde_yaml::Error> {
    sanitize_serde_yaml_value(&serde_yaml::Value::String(value.to_string()))
}

fn sanitize_serde_yaml_value(value: &serde_yaml::Value) -> Result<String, serde_yaml::Error> {
    let serialized = serde_yaml::to_string(value)?;
    let without_document_start = serialized
        .strip_prefix("---\n")
        .or_else(|| serialized.strip_prefix("---"))
        .unwrap_or(&serialized);

    Ok(without_document_start.trim_end().to_string())
}
