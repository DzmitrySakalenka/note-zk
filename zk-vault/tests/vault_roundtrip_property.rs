use std::{collections::BTreeMap, fs, path::Path};

use quickcheck::{Arbitrary, Gen};
use quickcheck_macros::quickcheck;
use tempfile::TempDir;
use zk_domain::{
    config::EffectiveConfig,
    id::{NoteId, RelativePath},
    note::NoteFrontmatter,
};
use zk_vault::{
    analyze_body, normalize_frontmatter_with_id_generator, parse_note_file, render_note_file,
    serialize_frontmatter,
};

#[derive(Clone, Debug)]
struct ArbitraryFrontmatter {
    id: String,
    title: String,
    tags: Vec<String>,
    note_type: Option<String>,
    aliases: Vec<String>,
    created: Option<String>,
    updated: Option<String>,
    source: Option<String>,
    status: Option<String>,
    sources: Vec<String>,
    extra: Vec<(String, String)>,
}

#[derive(Clone, Debug)]
struct ArbitraryHostileFrontmatter {
    id: String,
    title: String,
    tags: Vec<String>,
    note_type: Option<String>,
    aliases: Vec<String>,
    created: Option<String>,
    updated: Option<String>,
    source: Option<String>,
    status: Option<String>,
    sources: Vec<String>,
    extra: Vec<(String, String)>,
}

impl ArbitraryFrontmatter {
    fn into_note_frontmatter(self) -> NoteFrontmatter {
        let mut extra = BTreeMap::new();
        for (key, value) in self.extra {
            extra.insert(key, serde_yaml::Value::String(value));
        }

        NoteFrontmatter {
            id: NoteId::from(self.id.as_str()),
            title: self.title,
            tags: self.tags,
            note_type: self.note_type,
            aliases: self.aliases,
            created: self.created,
            updated: self.updated,
            source: self.source,
            status: self.status,
            sources: self
                .sources
                .into_iter()
                .map(|value| NoteId::from(value.as_str()))
                .collect(),
            extra,
        }
    }
}

impl ArbitraryHostileFrontmatter {
    fn into_note_frontmatter(self) -> NoteFrontmatter {
        let mut extra = BTreeMap::new();
        for (key, value) in self.extra {
            extra.insert(key, serde_yaml::Value::String(value));
        }

        NoteFrontmatter {
            id: NoteId::from(self.id.as_str()),
            title: self.title,
            tags: self.tags,
            note_type: self.note_type,
            aliases: self.aliases,
            created: self.created,
            updated: self.updated,
            source: self.source,
            status: self.status,
            sources: self
                .sources
                .into_iter()
                .map(|value| NoteId::from(value.as_str()))
                .collect(),
            extra,
        }
    }
}

impl Arbitrary for ArbitraryFrontmatter {
    fn arbitrary(g: &mut Gen) -> Self {
        Self {
            id: prefixed_token(g, "id", 24),
            title: prefixed_token(g, "title", 24),
            tags: token_vec(g, "tag", 1, 4, 16),
            note_type: maybe_token(g, "type", 16),
            aliases: token_vec(g, "alias", 0, 4, 16),
            created: maybe_token(g, "created", 20),
            updated: maybe_token(g, "updated", 20),
            source: maybe_token(g, "source", 20),
            status: maybe_token(g, "status", 16),
            sources: token_vec(g, "src", 0, 3, 20),
            extra: pair_vec(g, "xk", "xv", 0, 4, 14),
        }
    }
}

impl Arbitrary for ArbitraryHostileFrontmatter {
    fn arbitrary(g: &mut Gen) -> Self {
        Self {
            id: prefixed_token(g, "id", 24),
            title: hostile_token(g, "title", 24),
            tags: hostile_token_vec(g, "tag", 1, 4, 16),
            note_type: Some(hostile_token(g, "type", 16)),
            aliases: hostile_token_vec(g, "alias", 1, 4, 16),
            created: Some(hostile_token(g, "created", 20)),
            updated: Some(hostile_token(g, "updated", 20)),
            source: Some(hostile_token(g, "source", 20)),
            status: Some(hostile_token(g, "status", 16)),
            sources: token_vec(g, "src", 1, 3, 20),
            extra: pair_vec(g, "xk", "xv", 0, 4, 14),
        }
    }
}

fn prefixed_token(g: &mut Gen, prefix: &str, max_len: usize) -> String {
    let raw_len = usize::arbitrary(g) % max_len;
    let token_len = raw_len + 1;
    let alphabet = b"abcdefghijklmnopqrstuvwxyz0123456789_-";
    let mut token = String::from(prefix);
    token.push('_');
    for _ in 0..token_len {
        let index = usize::arbitrary(g) % alphabet.len();
        token.push(alphabet[index] as char);
    }
    token
}

fn hostile_token(g: &mut Gen, prefix: &str, max_len: usize) -> String {
    const HOSTILE_SUFFIXES: [&str; 10] = [
        ": value",
        "- leading",
        ": leading",
        "#tagless",
        " [list]",
        " {map}",
        " \"quoted\"",
        r" \backslash",
        "\nline2",
        " !bang",
    ];

    let base = prefixed_token(g, prefix, max_len);
    let suffix = HOSTILE_SUFFIXES[usize::arbitrary(g) % HOSTILE_SUFFIXES.len()];
    format!("{base}{suffix}")
}

fn maybe_token(g: &mut Gen, prefix: &str, max_len: usize) -> Option<String> {
    if bool::arbitrary(g) {
        Some(prefixed_token(g, prefix, max_len))
    } else {
        None
    }
}

fn hostile_token_vec(
    g: &mut Gen,
    prefix: &str,
    min_items: usize,
    max_items_exclusive: usize,
    max_len: usize,
) -> Vec<String> {
    let item_count = min_items + (usize::arbitrary(g) % max_items_exclusive);
    (0..item_count)
        .map(|_| hostile_token(g, prefix, max_len))
        .collect()
}

fn token_vec(
    g: &mut Gen,
    prefix: &str,
    min_items: usize,
    max_items_exclusive: usize,
    max_len: usize,
) -> Vec<String> {
    let item_count = min_items + (usize::arbitrary(g) % max_items_exclusive);
    (0..item_count)
        .map(|_| prefixed_token(g, prefix, max_len))
        .collect()
}

fn pair_vec(
    g: &mut Gen,
    key_prefix: &str,
    value_prefix: &str,
    min_items: usize,
    max_items_exclusive: usize,
    max_len: usize,
) -> Vec<(String, String)> {
    let item_count = min_items + (usize::arbitrary(g) % max_items_exclusive);
    (0..item_count)
        .map(|_| {
            (
                prefixed_token(g, key_prefix, max_len),
                prefixed_token(g, value_prefix, max_len),
            )
        })
        .collect()
}

fn write_text_file(path: &Path, content: &str) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("parent directory should be created");
    }
    fs::write(path, content).expect("file should be written");
}

fn roundtrip_preserves_data(original_frontmatter: NoteFrontmatter) -> bool {
    let original_serialized = serialize_frontmatter(&original_frontmatter);
    let original_body = "property_body_line\n";
    let original_file = render_note_file(&original_frontmatter, original_body);

    let tempdir = TempDir::new().expect("tempdir should be created");
    let note_path = tempdir.path().join("property-note.md");
    write_text_file(&note_path, &original_file);

    let parsed = parse_note_file(&note_path).expect("rendered note should parse");
    let raw_frontmatter = parsed.frontmatter.expect("frontmatter should exist");
    let body_analysis = analyze_body(&parsed.body_markdown);
    let config = EffectiveConfig::default();
    let relative_path = RelativePath::new("property-note.md");
    let context = zk_vault::NormalizationContext {
        relative_path: &relative_path,
        body_analysis: &body_analysis,
        config: &config,
        is_immutable: false,
    };
    let (normalized_frontmatter, _diagnostics) =
        normalize_frontmatter_with_id_generator(raw_frontmatter, &context, || {
            NoteId::from("01UNUSEDPROPERTYGENERATORID0001")
        });
    let normalized_serialized = serialize_frontmatter(&normalized_frontmatter);

    normalized_frontmatter == original_frontmatter && normalized_serialized == original_serialized
}

#[quickcheck]
fn property_frontmatter_roundtrip_preserves_data(case: ArbitraryFrontmatter) -> bool {
    roundtrip_preserves_data(case.into_note_frontmatter())
}

#[quickcheck]
fn property_frontmatter_roundtrip_preserves_yaml_hostile_scalars(
    case: ArbitraryHostileFrontmatter,
) -> bool {
    roundtrip_preserves_data(case.into_note_frontmatter())
}
