use bevy::ecs::hierarchy::ChildSpawnerCommands;
use bevy::prelude::*;
use unicode_segmentation::UnicodeSegmentation;

use crate::types::{EmojiAtlas, UiFonts};

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

#[derive(Clone, Debug)]
struct UiSegment {
    text: Vec<UiTextSpan>,
    emoji: Option<Handle<Image>>,
}

fn split_text_for_rich_ui(
    text: &str,
    emoji_atlas: &EmojiAtlas,
    asset_server: &AssetServer,
) -> Vec<UiSegment> {
    let mut segments = Vec::new();
    let mut pending_text: Vec<UiTextSpan> = Vec::new();

    for grapheme in text.graphemes(true) {
        let stripped = grapheme.replace('\u{fe0f}', "");
        let is_emoji = emojis::get(grapheme).is_some() || emojis::get(stripped.as_str()).is_some();
        if is_emoji {
            if let Some(emoji_handle) = emoji_atlas.lookup(grapheme, asset_server) {
                if !pending_text.is_empty() {
                    segments.push(UiSegment {
                        text: std::mem::take(&mut pending_text),
                        emoji: None,
                    });
                }
                segments.push(UiSegment {
                    text: Vec::new(),
                    emoji: Some(emoji_handle),
                });
                continue;
            }
        }

        let font = if is_emoji {
            UiFontKind::Emoji
        } else {
            UiFontKind::Cjk
        };
        if let Some(last) = pending_text.last_mut() {
            if last.font == font {
                last.text.push_str(grapheme);
                continue;
            }
        }
        pending_text.push(UiTextSpan {
            text: grapheme.to_string(),
            font,
        });
    }

    if !pending_text.is_empty() {
        segments.push(UiSegment {
            text: pending_text,
            emoji: None,
        });
    }

    segments
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

fn spawn_text_spans(
    parent: &mut ChildSpawnerCommands,
    spans: Vec<UiTextSpan>,
    fonts: &UiFonts,
    font_size: f32,
    color: Color,
    shadow: Option<TextShadow>,
) {
    let (base_text, base_font, rest) = split_base_span(spans);
    let mut entity = parent.spawn((
        Text::new(base_text),
        TextFont {
            font: font_handle(fonts, base_font),
            font_size,
            ..default()
        },
        TextColor(color),
    ));
    if let Some(shadow) = shadow {
        entity.insert(shadow);
    }

    if rest.is_empty() {
        return;
    }

    entity.with_children(|child| {
        for span in rest {
            let mut span_entity = child.spawn((
                TextSpan::new(span.text),
                TextFont {
                    font: font_handle(fonts, span.font),
                    font_size,
                    ..default()
                },
                TextColor(color),
            ));
            if let Some(shadow) = shadow {
                span_entity.insert(shadow);
            }
        }
    });
}

pub(crate) fn spawn_rich_text_line<B: Bundle>(
    parent: &mut ChildSpawnerCommands,
    text: &str,
    fonts: &UiFonts,
    emoji_atlas: &EmojiAtlas,
    asset_server: &AssetServer,
    font_size: f32,
    color: Color,
    extra: B,
    shadow: Option<TextShadow>,
) {
    let segments = split_text_for_rich_ui(text, emoji_atlas, asset_server);
    parent
        .spawn((extra, BackgroundColor(Color::NONE)))
        .with_children(|row| {
            for segment in segments {
                if let Some(handle) = segment.emoji {
                    let size = (font_size * 1.1).round().max(10.0);
                    row.spawn((
                        Node {
                            width: Val::Px(size),
                            height: Val::Px(size),
                            margin: UiRect::right(Val::Px(2.0)),
                            ..default()
                        },
                        ImageNode::new(handle),
                    ));
                } else if !segment.text.is_empty() {
                    spawn_text_spans(row, segment.text, fonts, font_size, color, shadow);
                }
            }
        });
}

pub(crate) fn set_rich_text_line(
    commands: &mut Commands,
    entity: Entity,
    text: &str,
    fonts: &UiFonts,
    emoji_atlas: &EmojiAtlas,
    asset_server: &AssetServer,
    font_size: f32,
    color: Color,
    shadow: Option<TextShadow>,
) {
    commands.entity(entity).despawn_children();
    let segments = split_text_for_rich_ui(text, emoji_atlas, asset_server);
    commands.entity(entity).with_children(|row| {
        for segment in segments {
            if let Some(handle) = segment.emoji {
                let size = (font_size * 1.1).round().max(10.0);
                row.spawn((
                    Node {
                        width: Val::Px(size),
                        height: Val::Px(size),
                        margin: UiRect::right(Val::Px(2.0)),
                        ..default()
                    },
                    ImageNode::new(handle),
                ));
            } else if !segment.text.is_empty() {
                spawn_text_spans(row, segment.text, fonts, font_size, color, shadow);
            }
        }
    });
}
