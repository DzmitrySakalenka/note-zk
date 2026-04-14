use zk_vault::{BodyAnalysis, analyze_body, detect_mermaid, extract_first_h1};

#[test]
fn extract_h1_when_present() {
    let body = "# Title\n\nContent";
    assert_eq!(extract_first_h1(body), Some("Title".to_string()));
}

#[test]
fn extract_h1_none_when_absent() {
    let body = "## Section\nNo level-1 heading";
    assert_eq!(extract_first_h1(body), None);
}

#[test]
fn extract_first_of_multiple_h1s() {
    let body = "# First\n\n# Second\n";
    assert_eq!(extract_first_h1(body), Some("First".to_string()));
}

#[test]
fn extract_h1_with_leading_indentation_up_to_three_spaces() {
    let body = "   # Indented H1\n";
    assert_eq!(extract_first_h1(body), Some("Indented H1".to_string()));
}

#[test]
fn extract_h1_ignores_h1_inside_fenced_code_block() {
    let body = "```md\n# Not a real heading\n```\n# Real heading\n";
    assert_eq!(extract_first_h1(body), Some("Real heading".to_string()));
}

#[test]
fn extract_h1_preserves_inline_formatting_and_trims_trailing_whitespace() {
    let body = "# Title with **bold** and `code`   \n";
    assert_eq!(
        extract_first_h1(body),
        Some("Title with **bold** and `code`".to_string())
    );
}

#[test]
fn detect_mermaid_true_for_basic_block() {
    let body = "```mermaid\ngraph TD\nA --> B\n```\n";
    assert!(detect_mermaid(body));
}

#[test]
fn detect_mermaid_false_when_absent() {
    let body = "```rust\nfn main() {}\n```\n";
    assert!(!detect_mermaid(body));
}

#[test]
fn detect_mermaid_true_with_whitespace_and_indented_opening_fence() {
    let body = "   ```mermaid   \ngraph LR\nX --> Y\n```\n";
    assert!(detect_mermaid(body));
}

#[test]
fn detect_mermaid_false_for_similar_info_strings() {
    let body = "```mermaidish\ngraph LR\nX --> Y\n```\n";
    assert!(!detect_mermaid(body));
}

#[test]
fn empty_body_returns_none_and_false() {
    let analysis = analyze_body("");
    assert_eq!(
        analysis,
        BodyAnalysis {
            first_h1: None,
            has_mermaid: false,
        }
    );
}

#[test]
fn body_with_only_code_blocks_returns_none_and_mermaid_as_expected() {
    let body = "````\n# no heading outside\n````\n```mermaid\ngraph TD\nA --> B\n```\n";
    let analysis = analyze_body(body);
    assert_eq!(analysis.first_h1, None);
    assert!(analysis.has_mermaid);
}

#[test]
fn analyze_body_combines_h1_and_mermaid_results() {
    let body = "# Heading\n\n```mermaid\ngraph TD\nA --> B\n```\n";
    let analysis = analyze_body(body);
    assert_eq!(
        analysis,
        BodyAnalysis {
            first_h1: Some("Heading".to_string()),
            has_mermaid: true,
        }
    );
}
