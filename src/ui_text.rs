use bevy::ecs::hierarchy::ChildSpawnerCommands;
use bevy::prelude::*;
use unicode_segmentation::UnicodeSegmentation;

use crate::types::UiFonts;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum UiFontKind {
    Cjk,
    Emoji,
}

#[derive(Clone, Debug)]
struct UiTextSpan {
    text: String,
    font: UiFontKind,
}

fn split_text_for_ui_fonts(text: &str) -> Vec<UiTextSpan> {
    let mut spans: Vec<UiTextSpan> = Vec::new();
    for grapheme in text.graphemes(true) {
        let font = if emojis::get(grapheme).is_some() {
            UiFontKind::Emoji
        } else {
            UiFontKind::Cjk
        };
        if let Some(last) = spans.last_mut() {
            if last.font == font {
                last.text.push_str(grapheme);
                continue;
            }
        }
        spans.push(UiTextSpan {
            text: grapheme.to_string(),
            font,
        });
    }
    spans
}

fn font_handle(fonts: &UiFonts, kind: UiFontKind) -> Handle<Font> {
    match kind {
        UiFontKind::Cjk => fonts.cjk.clone(),
        UiFontKind::Emoji => fonts.emoji.clone(),
    }
}

fn split_base_span(mut spans: Vec<UiTextSpan>) -> (String, UiFontKind, Vec<UiTextSpan>) {
    if spans.is_empty() {
        return (String::new(), UiFontKind::Cjk, spans);
    }
    let first = spans.remove(0);
    (first.text, first.font, spans)
}

pub(crate) fn spawn_text_with_ui_fonts<B: Bundle>(
    parent: &mut ChildSpawnerCommands,
    text: &str,
    fonts: &UiFonts,
    font_size: f32,
    color: Color,
    extra: B,
) {
    let (base_text, base_font, spans) = split_base_span(split_text_for_ui_fonts(text));

    let mut entity = parent.spawn((
        extra,
        Text::new(base_text),
        TextFont {
            font: font_handle(fonts, base_font),
            font_size,
            ..default()
        },
        TextColor(color),
    ));

    if spans.is_empty() {
        return;
    }

    entity.with_children(|parent| {
        for span in spans {
            parent.spawn((
                TextSpan::new(span.text),
                TextFont {
                    font: font_handle(fonts, span.font),
                    font_size,
                    ..default()
                },
                TextColor(color),
            ));
        }
    });
}

pub(crate) fn set_text_with_ui_fonts(
    commands: &mut Commands,
    text_entity: Entity,
    text: &str,
    fonts: &UiFonts,
    font_size: f32,
    color: Color,
) {
    let (base_text, base_font, spans) = split_base_span(split_text_for_ui_fonts(text));

    let mut entity = commands.entity(text_entity);
    entity.insert((
        Text::new(base_text),
        TextFont {
            font: font_handle(fonts, base_font),
            font_size,
            ..default()
        },
        TextColor(color),
    ));
    entity.despawn_children();

    if spans.is_empty() {
        return;
    }

    entity.with_children(|parent| {
        for span in spans {
            parent.spawn((
                TextSpan::new(span.text),
                TextFont {
                    font: font_handle(fonts, span.font),
                    font_size,
                    ..default()
                },
                TextColor(color),
            ));
        }
    });
}
