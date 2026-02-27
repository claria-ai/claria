use tera::{Context, Tera};

use claria_core::models::answer::SchematizedAnswer;

use crate::error::ExportError;

/// Render a Tera template with a SchematizedAnswer.
///
/// The `template_content` is the raw template string (Jinja2 syntax).
/// The `answer` fields become the template context variables.
pub fn render_template(
    template_name: &str,
    template_content: &str,
    answer: &SchematizedAnswer,
) -> Result<String, ExportError> {
    let mut tera = Tera::default();
    tera.add_raw_template(template_name, template_content)
        .map_err(|e| ExportError::TemplateParse(e.to_string()))?;

    // Convert the answer to a Tera context via serde_json
    let value = serde_json::to_value(answer)?;
    let context = Context::from_value(value)
        .map_err(|e| ExportError::TemplateRender(e.to_string()))?;

    let rendered = tera.render(template_name, &context)?;
    Ok(rendered)
}
