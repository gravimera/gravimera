#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ImportConflictPolicy {
    Replace,
    KeepBoth,
}

pub(crate) fn prompt_import_conflict_policy(
    title: &str,
    description: &str,
) -> Result<Option<ImportConflictPolicy>, String> {
    let title = title.trim();
    let description = description.trim();
    let result = rfd::MessageDialog::new()
        .set_level(rfd::MessageLevel::Warning)
        .set_title(if title.is_empty() {
            "Import conflicts detected"
        } else {
            title
        })
        .set_description(if description.is_empty() {
            "The selected import conflicts with existing content."
        } else {
            description
        })
        .set_buttons(rfd::MessageButtons::YesNoCancelCustom(
            "Replace".to_string(),
            "Keep Both".to_string(),
            "Quit".to_string(),
        ))
        .show();

    match result {
        rfd::MessageDialogResult::Custom(choice) => match choice.as_str() {
            "Replace" => Ok(Some(ImportConflictPolicy::Replace)),
            "Keep Both" => Ok(Some(ImportConflictPolicy::KeepBoth)),
            "Quit" => Ok(None),
            other => Err(format!("Unexpected import conflict choice: {other}")),
        },
        rfd::MessageDialogResult::Cancel => Ok(None),
        other => Err(format!("Unexpected import conflict dialog result: {other}")),
    }
}
