//! Starter mod layout for `octos new`.
//!
//! Matches the original `CreateNewWallpaper`: only create `octos.json` and
//! `index.html` when they are missing, so running `new` in an existing mod
//! folder never overwrites the author's files.

use std::fs;
use std::io;
use std::path::Path;

use serde_json::json;

pub fn create_mod_template(folder: &Path) -> io::Result<()> {
    let config = folder.join("octos.json");
    if !config.exists() {
        let name = folder
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("new-mod");
        let bytes = serde_json::to_vec_pretty(&json!({
            "name": name,
            "entry": "index.html",
        }))
        .map_err(io::Error::other)?;
        fs::write(config, bytes)?;
    }

    let entry = folder.join("index.html");
    if !entry.exists() {
        fs::write(entry, entry_html(folder))?;
    }
    Ok(())
}

fn entry_html(folder: &Path) -> String {
    format!(
        "<!DOCTYPE html>\n\
         <html lang=\"en\">\n\
         <head>\n\
         \t<meta charset=\"UTF-8\">\n\
         \t<title>My Mod</title>\n\
         \t<style>html, body {{ margin: 0; user-select: none; width: 100%; height: 100%; background-color: #e52e2e; overflow: hidden; text-align: center; align-content: center; font-family: \"Segoe UI\"; color: white; }}</style>\n\
         </head>\n\
         <body>\n\
         \t<h1>Hello, world!</h1>\n\
         \t<h3>You can edit me at {}</h3>\n\
         </body>\n\
         </html>\n",
        folder.display()
    )
}

#[cfg(test)]
mod tests {
    use tempfile::tempdir;

    use super::*;

    #[test]
    fn creates_config_and_entry() {
        let temp = tempdir().unwrap();
        let folder = temp.path().join("my-mod");
        fs::create_dir(&folder).unwrap();

        create_mod_template(&folder).unwrap();

        let config: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(folder.join("octos.json")).unwrap()).unwrap();
        assert_eq!(config["name"], "my-mod");
        assert_eq!(config["entry"], "index.html");
        let html = fs::read_to_string(folder.join("index.html")).unwrap();
        assert!(html.contains("Hello, world!"));
        assert!(html.contains(&folder.display().to_string()));
    }

    #[test]
    fn never_overwrites_existing_files() {
        let temp = tempdir().unwrap();
        let folder = temp.path().join("mod");
        fs::create_dir(&folder).unwrap();
        fs::write(folder.join("octos.json"), r#"{"name":"custom"}"#).unwrap();
        fs::write(folder.join("index.html"), "custom page").unwrap();

        create_mod_template(&folder).unwrap();

        assert_eq!(
            fs::read_to_string(folder.join("octos.json")).unwrap(),
            r#"{"name":"custom"}"#
        );
        assert_eq!(
            fs::read_to_string(folder.join("index.html")).unwrap(),
            "custom page"
        );
    }
}
