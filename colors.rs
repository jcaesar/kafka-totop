use tui::style::Color;

use crate::uses::*;

pub struct ColorAssignment {
    inner: HashMap<String, LineColor>,
}

impl ColorAssignment {
    pub fn new() -> Self {
        Self {
            inner: HashMap::new(),
        }
    }

    pub fn compute(&mut self, basestats: &[stats::TopicStats<'_>]) {
        let mut colors = LineColor::all_variants();
        let mut topdogs = basestats
            .iter()
            .take(colors.len())
            .filter(|s| s.seen > 0)
            .map(|s| s.topic)
            .collect::<HashSet<_>>();
        self.inner
            .retain(|topic: &String, _| topdogs.contains(topic.as_str()));
        for color in self.inner.values() {
            colors.remove(color);
        }
        let mut colors = colors.drain();
        for dog in topdogs.drain() {
            // TODO: It's high time for interned topic string names
            self.inner
                .entry(dog.to_string())
                .or_insert_with(|| colors.next().expect("no more dogs than colors"));
        }
    }

    pub fn get(&self, topic: &str) -> Color {
        self.inner
            .get(topic)
            .map(|c| (*c).into())
            .unwrap_or(Color::Gray)
    }

    pub fn len(&self) -> usize {
        self.inner.len()
    }
}

// Bit stupid to duplicate this here, but Color can't be inserted into a hash map
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum LineColor {
    Blue,
    Yellow,
    Red,
    Green,
    Magenta,
    Cyan,
    White,
}

impl Into<Color> for LineColor {
    fn into(self) -> Color {
        match self {
            LineColor::Blue => Color::Blue,
            LineColor::Yellow => Color::Yellow,
            LineColor::Red => Color::Red,
            LineColor::Green => Color::Green,
            LineColor::Magenta => Color::Magenta,
            LineColor::Cyan => Color::Cyan,
            LineColor::White => Color::White,
        }
    }
}
impl LineColor {
    pub(crate) fn all_variants() -> HashSet<Self> {
        use LineColor::*;
        [Blue, Yellow, Red, Green, Magenta, Cyan, White]
            .iter()
            .map(Clone::clone)
            .collect()
    }
}
