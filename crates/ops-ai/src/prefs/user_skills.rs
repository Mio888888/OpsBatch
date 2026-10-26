//! User skills: directories with a `SKILL.md` (YAML frontmatter `name` / `description`) in the app's skills folder
//! (`userSkills.cjs`). Settings lists them with their warnings; the slash picker offers the ready ones.

use std::path::{Path, PathBuf};

pub const DIR_NAME: &str = "skills";
pub const README_NAME: &str = "README.txt";
const MAX_SKILL_BYTES: u64 = 256 * 1024;
const MAX_DESCRIPTION_LENGTH: usize = 1024;

const README: &str = "OpsBatch user skills\n\nPut one directory per skill in this folder. Each directory needs a SKILL.md.\n\nExample layout:\n  skills/\n    My Skill/\n      SKILL.md\n\nMinimal SKILL.md:\n  ---\n  name: My Skill\n  description: Short summary of what this skill helps with.\n  ---\n\n  Write the skill instructions here.\n\nAfter adding or editing a skill, reopen the AI settings page or start a new chat to refresh the list.\n";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserSkill {
    pub id: String,
    pub slug: String,
    pub directory_name: String,
    pub directory_path: PathBuf,
    pub skill_path: PathBuf,
    pub name: String,
    pub description: String,
    pub ready: bool,
    pub warnings: Vec<String>,
    /// The instructions after the frontmatter (only read for skills without warnings).
    pub body: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SkillsStatus {
    pub directory: PathBuf,
    pub ready_count: usize,
    pub warning_count: usize,
    pub skills: Vec<UserSkill>,
}

pub fn skills_dir(data_dir: &Path) -> PathBuf {
    data_dir.join(DIR_NAME)
}

/// Same slug rule as quick messages: lower-case ASCII words joined by `-`.
pub fn slugify(value: &str) -> String {
    super::quick::normalize_slug(value)
}

fn strip_quotes(value: &str) -> String {
    let v = value.trim();
    let quoted = v.len() >= 2 && ((v.starts_with('"') && v.ends_with('"')) || (v.starts_with('\'') && v.ends_with('\'')));
    if quoted { v[1..v.len() - 1].to_string() } else { v.to_string() }
}

/// `(attributes, has_frontmatter)`
pub fn parse_frontmatter(content: &str) -> (std::collections::BTreeMap<String, String>, bool) {
    let mut attributes = std::collections::BTreeMap::new();
    let normalized = content.replace("\r\n", "\n");
    let Some(rest) = normalized.strip_prefix("---\n") else { return (attributes, false) };
    let Some(end) = rest.find("\n---") else { return (attributes, false) };
    for raw in rest[..end].lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(colon) = line.find(':').filter(|i| *i > 0) {
            attributes.insert(line[..colon].trim().to_string(), strip_quotes(&line[colon + 1..]));
        }
    }
    (attributes, true)
}

/// The text after the frontmatter block.
pub fn skill_body(content: &str) -> String {
    let normalized = content.replace("\r\n", "\n");
    match normalized.strip_prefix("---\n").and_then(|rest| rest.find("\n---").map(|end| &rest[end + 4..])) {
        Some(body) => body.trim_start_matches('\n').trim().to_string(),
        None => normalized.trim().to_string(),
    }
}

/// Creates the folder (with a README when it is empty) and returns it.
pub fn ensure_dir(data_dir: &Path) -> std::io::Result<PathBuf> {
    let dir = skills_dir(data_dir);
    std::fs::create_dir_all(&dir)?;
    if std::fs::read_dir(&dir)?.next().is_none() {
        std::fs::write(dir.join(README_NAME), README)?;
    }
    Ok(dir)
}

pub fn scan(data_dir: &Path) -> Result<SkillsStatus, String> {
    let dir = ensure_dir(data_dir).map_err(|e| e.to_string())?;
    let mut skills: Vec<UserSkill> = Vec::new();
    let mut entries: Vec<_> = std::fs::read_dir(&dir).map_err(|e| e.to_string())?.flatten().collect();
    entries.sort_by_key(|e| e.file_name());
    for entry in entries {
        let Ok(file_type) = entry.file_type() else { continue };
        // real directories only: symlinks could point anywhere
        if !file_type.is_dir() || file_type.is_symlink() {
            continue;
        }
        let dir_name = entry.file_name().to_string_lossy().into_owned();
        let skill_dir = entry.path();
        let skill_path = skill_dir.join("SKILL.md");
        let mut item = UserSkill { id: dir_name.clone(), slug: slugify(&dir_name), directory_name: dir_name.clone(), directory_path: skill_dir, skill_path: skill_path.clone(), name: dir_name.clone(), description: String::new(), ready: false, warnings: Vec::new(), body: String::new() };
        let meta = match std::fs::symlink_metadata(&skill_path) {
            Ok(m) => m,
            Err(_) => {
                item.warnings.push("Missing SKILL.md".into());
                skills.push(item);
                continue;
            }
        };
        if meta.file_type().is_symlink() {
            item.warnings.push("SKILL.md must not be a symbolic link.".into());
        } else if !meta.is_file() {
            item.warnings.push("SKILL.md must be a regular file.".into());
        } else if meta.len() > MAX_SKILL_BYTES {
            item.warnings.push(format!("SKILL.md is too large ({} bytes > {MAX_SKILL_BYTES} bytes).", meta.len()));
        }
        if !item.warnings.is_empty() {
            skills.push(item);
            continue;
        }
        match std::fs::read_to_string(&skill_path) {
            Ok(content) => {
                let (attrs, has_frontmatter) = parse_frontmatter(&content);
                let name = attrs.get("name").map(|s| s.trim().to_string()).unwrap_or_default();
                let description = attrs.get("description").map(|s| s.trim().to_string()).unwrap_or_default();
                let slug = slugify(if name.is_empty() { &dir_name } else { &name });
                if !has_frontmatter {
                    item.warnings.push("Missing YAML frontmatter.".into());
                }
                if name.is_empty() {
                    item.warnings.push("Missing frontmatter field: name.".into());
                }
                if description.is_empty() {
                    item.warnings.push("Missing frontmatter field: description.".into());
                } else if description.chars().count() > MAX_DESCRIPTION_LENGTH {
                    item.warnings.push(format!("Description is too long ({} chars > {MAX_DESCRIPTION_LENGTH}).", description.chars().count()));
                }
                if slug.is_empty() {
                    item.warnings.push("Skill name must include ASCII letters or digits to generate a usable slug.".into());
                }
                item.slug = slug;
                item.name = if name.is_empty() { dir_name } else { name };
                item.description = description;
                item.ready = item.warnings.is_empty();
                if item.ready {
                    item.body = skill_body(&content);
                }
            }
            Err(e) => item.warnings.push(format!("Could not read SKILL.md: {e}")),
        }
        skills.push(item);
    }
    // two ready skills may not share a slug
    let mut count: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for s in skills.iter().filter(|s| s.ready && !s.slug.is_empty()) {
        *count.entry(s.slug.clone()).or_default() += 1;
    }
    for s in skills.iter_mut().filter(|s| s.ready && count.get(&s.slug).copied().unwrap_or(0) > 1) {
        s.ready = false;
        s.warnings.push(format!("Duplicate skill slug \"{}\". Rename the skill or change its frontmatter name.", s.slug));
    }
    let ready_count = skills.iter().filter(|s| s.ready).count();
    let warning_count = skills.len() - ready_count;
    Ok(SkillsStatus { directory: dir, ready_count, warning_count, skills })
}

/// Slugs the quick-message editor must not reuse.
pub fn reserved_slugs(status: &SkillsStatus) -> Vec<String> {
    status.skills.iter().filter(|s| s.ready).map(|s| s.slug.clone()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("opsbatch-skills-{name}-{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn skill(root: &Path, dir: &str, body: Option<&str>) {
        let d = skills_dir(root).join(dir);
        std::fs::create_dir_all(&d).unwrap();
        if let Some(body) = body {
            std::fs::write(d.join("SKILL.md"), body).unwrap();
        }
    }

    #[test]
    fn the_body_follows_the_frontmatter() {
        assert_eq!(skill_body("---\nname: x\n---\n\nDo this.\nThen that.\n"), "Do this.\nThen that.");
        assert_eq!(skill_body("plain"), "plain");
        let root = temp("body");
        skill(&root, "ok", Some("---\nname: Ok\ndescription: d\n---\nsteps here"));
        assert_eq!(scan(&root).unwrap().skills[0].body, "steps here");
    }

    #[test]
    fn an_empty_folder_gets_a_readme_and_no_skills() {
        let root = temp("empty");
        let status = scan(&root).unwrap();
        assert_eq!((status.ready_count, status.warning_count, status.skills.len()), (0, 0, 0));
        assert!(status.directory.join(README_NAME).exists());
        // the readme is not a skill and a non-empty folder keeps what it has
        assert_eq!(scan(&root).unwrap().skills.len(), 0);
    }

    #[test]
    fn frontmatter_is_parsed() {
        let (attrs, has) = parse_frontmatter("---\r\nname: \"My Skill\"\r\ndescription: 'does x: y'\r\n# c\r\n---\r\nbody");
        assert!(has);
        assert_eq!(attrs["name"], "My Skill");
        assert_eq!(attrs["description"], "does x: y");
        assert!(!parse_frontmatter("no frontmatter").1);
        assert!(!parse_frontmatter("---\nname: x\n").1);
    }

    #[test]
    fn ready_and_warning_skills_are_reported() {
        let root = temp("mixed");
        skill(&root, "Deploy Helper", Some("---\nname: Deploy Helper\ndescription: ships it\n---\nsteps"));
        skill(&root, "no-file", None);
        skill(&root, "no-front", Some("just text"));
        skill(&root, "no-desc", Some("---\nname: Thing\n---\nx"));
        skill(&root, "中文", Some("---\nname: 中文\ndescription: d\n---\nx"));
        let status = scan(&root).unwrap();
        let by = |dir: &str| status.skills.iter().find(|s| s.directory_name == dir).unwrap().clone();
        assert!(by("Deploy Helper").ready);
        assert_eq!(by("Deploy Helper").slug, "deploy-helper");
        assert_eq!(by("no-file").warnings, vec!["Missing SKILL.md"]);
        assert!(by("no-front").warnings.contains(&"Missing YAML frontmatter.".to_string()));
        assert_eq!(by("no-desc").warnings, vec!["Missing frontmatter field: description."]);
        assert!(by("中文").warnings.iter().any(|w| w.contains("usable slug")));
        assert_eq!((status.ready_count, status.warning_count), (1, 4));
        assert_eq!(reserved_slugs(&status), vec!["deploy-helper"]);
    }

    #[test]
    fn duplicate_slugs_downgrade_both() {
        let root = temp("dupes");
        skill(&root, "a", Some("---\nname: Same Name\ndescription: one\n---\n"));
        skill(&root, "b", Some("---\nname: same-name\ndescription: two\n---\n"));
        let status = scan(&root).unwrap();
        assert_eq!((status.ready_count, status.warning_count), (0, 2));
        assert!(status.skills.iter().all(|s| s.warnings.iter().any(|w| w.contains("Duplicate skill slug"))));
    }

    #[test]
    fn oversized_skills_are_not_read() {
        let root = temp("big");
        skill(&root, "huge", Some(&"x".repeat(MAX_SKILL_BYTES as usize + 1)));
        let status = scan(&root).unwrap();
        assert!(status.skills[0].warnings[0].starts_with("SKILL.md is too large"));
    }
}
