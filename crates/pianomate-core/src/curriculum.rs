//! How lessons are organised: goals are roads made of stages, stages draw on source books.

use std::collections::HashSet;

use serde::Deserialize;

#[derive(Clone, Debug, Deserialize)]
pub struct Curriculum {
    pub books: Vec<Book>,
    pub stages: Vec<Stage>,
    pub goals: Vec<Goal>,
}

/// A published method or collection that lessons are drawn from.
#[derive(Clone, Debug, Deserialize)]
pub struct Book {
    pub id: String,
    pub title: String,
    pub author: String,
    pub year: u16,
    pub image: String,
    pub blurb: String,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Stage {
    pub id: String,
    pub title: String,
    pub blurb: String,
    pub image: String,
    /// Ids of the books this stage draws on.
    #[serde(default)]
    pub books: Vec<String>,
    /// Lesson ids in order. Empty while the stage is still being written.
    #[serde(default)]
    pub lessons: Vec<String>,
    /// The part of the road this stage belongs to, shown as a heading.
    #[serde(default)]
    pub part: String,
    /// A music symbol drawn as the stage's picture instead of `image`: see `EMBLEMS`.
    #[serde(default)]
    pub emblem: String,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Goal {
    pub id: String,
    pub title: String,
    pub tagline: String,
    pub blurb: String,
    pub image: String,
    /// Stage ids in the order they are studied.
    pub stages: Vec<String>,
}

/// Stage emblems: a name and the SMuFL symbol drawn for it.
pub const EMBLEMS: [(&str, char); 12] = [
    ("hand", '\u{E650}'),
    ("treble", '\u{E050}'),
    ("bass", '\u{E062}'),
    ("quarter", '\u{E1D5}'),
    ("eighths", '\u{E1F0}'),
    ("rest", '\u{E4E5}'),
    ("staccato", '\u{E4A2}'),
    ("sharp", '\u{E262}'),
    ("flat", '\u{E260}'),
    ("chord", '\u{E0A2}'),
    ("fermata", '\u{E4C0}'),
    ("forte", '\u{E522}'),
];

pub fn emblem(name: &str) -> Option<char> {
    EMBLEMS.iter().find(|e| e.0 == name).map(|e| e.1)
}

impl Curriculum {
    /// Parses the curriculum and checks every cross-reference against the lessons that exist.
    pub fn parse(source: &str, lesson_ids: &[&str]) -> Result<Curriculum, String> {
        let c: Curriculum = ron::from_str(source).map_err(|e| e.to_string())?;
        let unique = |kind: &str, ids: Vec<&String>| {
            let mut seen = HashSet::new();
            ids.into_iter().find(|id| !seen.insert(*id)).map_or(Ok(()), |id| Err(format!("duplicate {kind} id {id:?}")))
        };
        unique("book", c.books.iter().map(|b| &b.id).collect())?;
        unique("stage", c.stages.iter().map(|s| &s.id).collect())?;
        unique("goal", c.goals.iter().map(|g| &g.id).collect())?;
        unique("lesson placement of", c.stages.iter().flat_map(|s| &s.lessons).collect())?;
        for stage in &c.stages {
            if let Some(id) = stage.books.iter().find(|id| c.book(id).is_none()) {
                return Err(format!("stage {:?} cites unknown book {id:?}", stage.id));
            }
            if let Some(id) = stage.lessons.iter().find(|id| !lesson_ids.contains(&id.as_str())) {
                return Err(format!("stage {:?} lists unknown lesson {id:?}", stage.id));
            }
            if !stage.emblem.is_empty() && emblem(&stage.emblem).is_none() {
                return Err(format!("stage {:?} has unknown emblem {:?}", stage.id, stage.emblem));
            }
        }
        for goal in &c.goals {
            if goal.stages.is_empty() {
                return Err(format!("goal {:?} has no stages", goal.id));
            }
            if let Some(id) = goal.stages.iter().find(|id| c.stage(id).is_none()) {
                return Err(format!("goal {:?} lists unknown stage {id:?}", goal.id));
            }
        }
        if c.goals.is_empty() {
            return Err("no goals".into());
        }
        Ok(c)
    }

    pub fn book(&self, id: &str) -> Option<&Book> {
        self.books.iter().find(|b| b.id == id)
    }

    pub fn stage(&self, id: &str) -> Option<&Stage> {
        self.stages.iter().find(|s| s.id == id)
    }

    pub fn goal(&self, id: &str) -> Option<&Goal> {
        self.goals.iter().find(|g| g.id == id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lesson::Lesson;

    const SAMPLE: &str = r#"(
        books: [(id: "b", title: "B", author: "A", year: 1873, image: "b.jpg", blurb: "")],
        stages: [(id: "s", title: "S", blurb: "", image: "b.jpg", books: ["b"], lessons: ["l1"], emblem: "treble")],
        goals: [(id: "g", title: "G", tagline: "", blurb: "", image: "g.jpg", stages: ["s"])],
    )"#;

    #[test]
    fn checks_references() {
        assert!(Curriculum::parse(SAMPLE, &["l1"]).is_ok());
        assert!(Curriculum::parse(SAMPLE, &[]).unwrap_err().contains("unknown lesson"));
        assert!(Curriculum::parse(&SAMPLE.replace(r#"books: ["b"]"#, r#"books: ["x"]"#), &["l1"]).unwrap_err().contains("unknown book"));
        assert!(Curriculum::parse(&SAMPLE.replace(r#"stages: ["s"]"#, r#"stages: ["x"]"#), &["l1"]).unwrap_err().contains("unknown stage"));
        assert!(Curriculum::parse(&SAMPLE.replace(r#"stages: ["s"]"#, "stages: []"), &["l1"]).is_err());
        assert!(Curriculum::parse(&SAMPLE.replace(r#"emblem: "treble""#, r#"emblem: "harp""#), &["l1"]).unwrap_err().contains("unknown emblem"));
    }

    #[test]
    fn shipped_curriculum_is_consistent() {
        let root = concat!(env!("CARGO_MANIFEST_DIR"), "/../../content");
        let lessons: Vec<Lesson> = std::fs::read_dir(format!("{root}/lessons"))
            .unwrap()
            .map(|e| Lesson::parse(&std::fs::read_to_string(e.unwrap().path()).unwrap()).unwrap())
            .collect();
        let ids: Vec<&str> = lessons.iter().map(|l| l.id.as_str()).collect();
        let c = Curriculum::parse(&std::fs::read_to_string(format!("{root}/curriculum.ron")).unwrap(), &ids).unwrap();
        for id in &ids {
            assert!(c.stages.iter().any(|s| s.lessons.iter().any(|l| l == id)), "lesson {id} is in no stage");
        }
        for goal in &c.goals {
            let parts: Vec<&str> = goal.stages.iter().map(|id| c.stage(id).unwrap().part.as_str()).collect();
            let mut seen = Vec::new();
            for (i, part) in parts.iter().enumerate() {
                assert!(i == 0 || parts[i - 1] == *part || !seen.contains(part), "goal {}: part {part:?} is split up", goal.id);
                seen.push(*part);
            }
        }
        for image in c.books.iter().map(|b| &b.image).chain(c.stages.iter().map(|s| &s.image)).chain(c.goals.iter().map(|g| &g.image)) {
            assert!(std::path::Path::new(&format!("{root}/images/{image}")).exists(), "missing picture {image}");
        }
    }
}
