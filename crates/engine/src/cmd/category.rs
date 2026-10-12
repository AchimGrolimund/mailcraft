//! Colour categories.

use mailcraft_model::Rgb;
use serde_json::{Value, json};

use super::{bad, done};
use crate::{CmdResult, CommandSpec, Session, p};

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec::new("category.set", "New Category", "Home › Tags", set).params(r##"{"name": string, "color": "#RRGGBB"}"##),
        CommandSpec::new("category.rename", "Rename Category", "Home › Tags", rename).params(r#"{"from": string, "to": string}"#),
        CommandSpec::new("category.remove", "Delete Category", "Home › Tags", remove).params(r#"{"name": string}"#),
        CommandSpec::new("category.list", "All Categories", "Home › Tags", list).pure(),
    ]
}

fn set(s: &mut Session, v: &Value) -> CmdResult {
    let color = Rgb::parse(p::req_str(v, "color")?).ok_or_else(|| bad("`color` looks like #2E8B57"))?;
    if s.mailbox.categories.len() >= 200 && s.mailbox.category(p::req_str(v, "name")?).is_none() {
        return Err(super::fail("there are too many categories"));
    }
    s.mailbox.set_category(p::req_str(v, "name")?, color)?;
    done()
}

fn rename(s: &mut Session, v: &Value) -> CmdResult {
    s.mailbox.rename_category(p::req_str(v, "from")?, p::req_str(v, "to")?)?;
    done()
}

fn remove(s: &mut Session, v: &Value) -> CmdResult {
    s.mailbox.remove_category(p::req_str(v, "name")?)?;
    done()
}

fn list(s: &mut Session, _: &Value) -> CmdResult {
    Ok(json!({"categories": s.mailbox.categories.iter().map(|c| json!({"name": c.name, "color": c.color.hex()})).collect::<Vec<_>>()}))
}
