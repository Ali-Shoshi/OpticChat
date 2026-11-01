use pulldown_cmark::{Event, Parser, Tag, TagEnd};

pub fn escape(text: &str) -> String {
    text.chars().flat_map(|c| {
        if "\\`*_{}[]<>()#+-.!|>".contains(c) { vec!['\\', c] } else { vec![c] }
    }).collect()
}

// Convert CommonMark into the subset supported by Slint's native text renderer.
// Unsupported blocks keep their readable contents rather than exposing markup.
pub fn formatted(text: &str) -> slint::StyledText {
    let mut out = String::new();
    for event in Parser::new(text) {
        match event {
            Event::Start(Tag::Strong | Tag::Heading { .. }) => out.push_str("**"),
            Event::End(TagEnd::Strong) => out.push_str("**"),
            Event::End(TagEnd::Heading(_)) => out.push_str("**\n\n"),
            Event::Start(Tag::Emphasis) | Event::End(TagEnd::Emphasis) => out.push('*'),
            Event::Start(Tag::Item) => out.push_str("• "),
            Event::End(TagEnd::Item) => out.push_str("\n\n"),
            Event::End(TagEnd::Paragraph | TagEnd::CodeBlock | TagEnd::BlockQuote(_)) => out.push_str("\n\n"),
            Event::Text(t) | Event::Code(t) | Event::Html(t) | Event::InlineHtml(t) => out.push_str(&escape(&t)),
            Event::SoftBreak | Event::HardBreak => out.push_str("  \n"),
            Event::Rule => out.push_str("\n\n────────\n\n"),
            _ => {}
        }
    }
    slint::StyledText::from_markdown(&out)
        .unwrap_or_else(|_| slint::StyledText::from_plain_text(text))
}

#[cfg(test)]
mod tests {
    #[test]
    fn renders_common_ai_markdown() {
        let rendered = super::formatted("## Next steps\n\nUse **Settings** and *save*.\n\n```rust\nlet x = 1;\n```");
        let expected = slint::StyledText::from_markdown("**Next steps**\n\nUse **Settings** and *save*.\n\nlet x = 1;\n\n\n").unwrap();
        assert_eq!(rendered, expected);
    }

}
