//! The static system map, parsed from the Zephyr devicetree.
//!
//! Zephyr already describes the whole board in `build/zephyr/zephyr.dts`: every
//! peripheral (I2C/SPI/UART/timers/GPIO) and every sensor/actuator declared on a
//! bus, with its address, binding (`compatible`) and on/off state. We parse that
//! into a compact map — "what's in this system" — for the human and the agent.
//! We don't invent the map; we read the one the build already produced.
//!
//! This is a focused devicetree reader: enough of the grammar (nodes, labels,
//! properties, the `reg`/`compatible`/`status`/`model` we care about) to build
//! the map, not a general DTS compiler.

/// A parsed devicetree node.
#[derive(Debug, Default, Clone)]
pub struct Node {
    pub labels: Vec<String>,
    pub name: String,
    pub unit_address: Option<String>,
    pub props: Vec<(String, String)>,
    pub children: Vec<Node>,
}

impl Node {
    fn prop(&self, key: &str) -> Option<&str> {
        self.props.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
    }

    /// First string of a (possibly list) `compatible` property.
    pub fn compatible(&self) -> Option<String> {
        first_string(self.prop("compatible")?)
    }

    /// Effective status; absent status means enabled ("okay") in devicetree.
    pub fn status(&self) -> &str {
        self.prop("status")
            .and_then(|v| first_string(v).map(|_| v))
            .map(|v| v.trim().trim_matches('"'))
            .unwrap_or("okay")
    }

    /// Base address: the first cell of `reg`, else the unit address.
    pub fn address(&self) -> Option<u64> {
        if let Some(reg) = self.prop("reg")
            && let Some(a) = first_number(reg)
        {
            return Some(a);
        }
        self.unit_address
            .as_deref()
            .and_then(|u| u64::from_str_radix(u, 16).ok())
    }
}

/// A parsed device for the rendered map.
struct Device {
    label: String,
    compatible: String,
    address: Option<u64>,
    indent: usize,
}

/// Parse a devicetree source string into its root node.
pub fn parse(dts: &str) -> Option<Node> {
    let stripped = strip_comments(dts);
    let mut p = Parser::new(&stripped);
    let (_props, top) = p.parse_body();
    // The root node is named "/".
    top.into_iter().find(|n| n.name == "/").or_else(|| {
        // Fall back to the first node if "/" wasn't found.
        let stripped2 = strip_comments(dts);
        Parser::new(&stripped2).parse_body().1.into_iter().next()
    })
}

/// Parse a devicetree and render the human/agent-readable system map.
pub fn render(dts: &str) -> Option<String> {
    let root = parse(dts)?;
    let model = root
        .prop("model")
        .and_then(first_string)
        .unwrap_or_else(|| "unknown board".to_string());

    let mut active: Vec<Device> = Vec::new();
    let mut disabled: Vec<String> = Vec::new();
    collect(&root, 0, &mut active, &mut disabled);

    let mut out = format!("System map — {model} (from devicetree)\n\nActive devices:\n");
    if active.is_empty() {
        out.push_str("  (none)\n");
    }
    for d in &active {
        let pad = "  ".repeat(d.indent + 1);
        let addr = d
            .address
            .map(|a| format!("  @{a:#010x}"))
            .unwrap_or_default();
        out.push_str(&format!("{pad}{:<22}{}{addr}\n", d.label, d.compatible));
    }
    if !disabled.is_empty() {
        out.push_str(&format!("\nDisabled: {}\n", disabled.join(", ")));
    }
    Some(out.trim_end().to_string())
}

/// Walk the tree, collecting active devices (and disabled names). Pin-mux
/// definitions are skipped (they are wiring noise, not devices), but we still
/// recurse so GPIO ports under the pin controller are found.
fn collect(node: &Node, indent: usize, active: &mut Vec<Device>, disabled: &mut Vec<String>) {
    let mut child_indent = indent;
    if let Some(compatible) = node.compatible() {
        let is_pinmux = compatible.contains("pinctrl") || node.prop("pinmux").is_some();
        if !is_pinmux {
            let label = if node.labels.is_empty() {
                display_name(node)
            } else {
                node.labels.join(" ")
            };
            if node.status() == "disabled" {
                disabled.push(label);
                return; // a disabled node's children are inactive too
            }
            active.push(Device {
                label,
                compatible,
                address: node.address(),
                indent,
            });
            child_indent = indent + 1;
        }
    }
    for child in &node.children {
        collect(child, child_indent, active, disabled);
    }
}

fn display_name(node: &Node) -> String {
    match &node.unit_address {
        Some(a) => format!("{}@{a}", node.name),
        None => node.name.clone(),
    }
}

// --- minimal devicetree parser -------------------------------------------

struct Parser {
    chars: Vec<char>,
    pos: usize,
}

impl Parser {
    fn new(s: &str) -> Self {
        Self { chars: s.chars().collect(), pos: 0 }
    }

    fn peek(&self) -> Option<char> {
        self.chars.get(self.pos).copied()
    }

    fn bump(&mut self) -> Option<char> {
        let c = self.peek();
        if c.is_some() {
            self.pos += 1;
        }
        c
    }

    fn skip_ws(&mut self) {
        while matches!(self.peek(), Some(c) if c.is_whitespace()) {
            self.pos += 1;
        }
    }

    /// Parse a node body: a sequence of properties and child nodes, until the
    /// closing `}` (consumed) or end of input. Returns (properties, children).
    fn parse_body(&mut self) -> (Vec<(String, String)>, Vec<Node>) {
        let mut props = Vec::new();
        let mut children = Vec::new();
        loop {
            self.skip_ws();
            match self.peek() {
                None => break,
                Some('}') => {
                    self.bump();
                    break;
                }
                _ => {
                    let (header, delim) = self.read_until_delim();
                    match delim {
                        '{' => {
                            let (cprops, cchildren) = self.parse_body();
                            self.skip_ws();
                            if self.peek() == Some(';') {
                                self.bump();
                            }
                            children.push(build_node(&header, cprops, cchildren));
                        }
                        ';' => {
                            if let Some(p) = parse_property(&header) {
                                props.push(p);
                            }
                        }
                        _ => break, // EOF
                    }
                }
            }
        }
        (props, children)
    }

    /// Read up to (and consuming) the next `{` or `;`, honoring string literals.
    /// Returns the text read and the delimiter ('\0' at EOF).
    fn read_until_delim(&mut self) -> (String, char) {
        let mut buf = String::new();
        let mut in_str = false;
        while let Some(c) = self.peek() {
            if in_str {
                buf.push(c);
                self.bump();
                if c == '"' {
                    in_str = false;
                }
                continue;
            }
            match c {
                '"' => {
                    in_str = true;
                    buf.push(c);
                    self.bump();
                }
                '{' | ';' => {
                    self.bump();
                    return (buf, c);
                }
                '}' => return (buf, '}'),
                _ => {
                    buf.push(c);
                    self.bump();
                }
            }
        }
        (buf, '\0')
    }
}

/// Build a node from its header ("label: label2: name@addr") and body.
fn build_node(header: &str, props: Vec<(String, String)>, children: Vec<Node>) -> Node {
    let mut labels = Vec::new();
    let mut name = String::new();
    for token in header.split_whitespace() {
        if let Some(label) = token.strip_suffix(':') {
            labels.push(label.to_string());
        } else {
            name = token.to_string();
        }
    }
    let (name, unit_address) = match name.split_once('@') {
        Some((n, a)) => (n.to_string(), Some(a.to_string())),
        None => (name, None),
    };
    Node { labels, name, unit_address, props, children }
}

/// Parse "key = value" (or a boolean "key") into a (key, value) pair.
fn parse_property(header: &str) -> Option<(String, String)> {
    let header = header.trim();
    if header.is_empty() {
        return None;
    }
    match header.split_once('=') {
        Some((k, v)) => Some((k.trim().to_string(), v.trim().to_string())),
        None => Some((header.to_string(), "true".to_string())),
    }
}

/// First quoted string in a property value, e.g. `"a", "b"` -> "a".
fn first_string(value: &str) -> Option<String> {
    let start = value.find('"')? + 1;
    let end = value[start..].find('"')? + start;
    Some(value[start..end].to_string())
}

/// First numeric cell in a `< ... >` value, e.g. `< 0x40005400 0x400 >`.
fn first_number(value: &str) -> Option<u64> {
    for tok in value.trim_matches(|c| c == '<' || c == '>').split_whitespace() {
        if let Some(hex) = tok.strip_prefix("0x").or_else(|| tok.strip_prefix("0X")) {
            if let Ok(n) = u64::from_str_radix(hex, 16) {
                return Some(n);
            }
        } else if let Ok(n) = tok.parse::<u64>() {
            return Some(n);
        }
    }
    None
}

/// Remove `/* ... */` and `// ...` comments, honoring string literals.
fn strip_comments(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    let mut in_str = false;
    while i < chars.len() {
        let c = chars[i];
        if in_str {
            out.push(c);
            if c == '"' {
                in_str = false;
            }
            i += 1;
            continue;
        }
        if c == '"' {
            in_str = true;
            out.push(c);
            i += 1;
        } else if c == '/' && chars.get(i + 1) == Some(&'*') {
            i += 2;
            while i < chars.len() && !(chars[i] == '*' && chars.get(i + 1) == Some(&'/')) {
                i += 1;
            }
            i += 2; // skip "*/"
        } else if c == '/' && chars.get(i + 1) == Some(&'/') {
            while i < chars.len() && chars[i] != '\n' {
                i += 1;
            }
        } else {
            out.push(c);
            i += 1;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"
/dts-v1/;
/ {
    model = "Demo Board";
    soc {
        /* a controller with a sensor on it */
        i2c1: arduino_i2c: i2c@40005400 {
            compatible = "st,stm32-i2c-v1";
            reg = < 0x40005400 0x400 >;
            status = "okay";
            bme280@76 {
                compatible = "bosch,bme280";
                reg = < 0x76 >;
            };
        };
        i2c2: i2c@40005800 {
            compatible = "st,stm32-i2c-v1";
            reg = < 0x40005800 0x400 >;
            status = "disabled";
        };
    };
};
"#;

    #[test]
    fn parses_model_and_nodes() {
        let root = parse(SAMPLE).unwrap();
        assert_eq!(root.name, "/");
        assert_eq!(root.prop("model").and_then(first_string).unwrap(), "Demo Board");
    }

    #[test]
    fn extracts_labels_address_and_compatible() {
        let root = parse(SAMPLE).unwrap();
        let soc = root.children.iter().find(|n| n.name == "soc").unwrap();
        let i2c1 = soc.children.iter().find(|n| n.labels.contains(&"i2c1".to_string())).unwrap();
        assert_eq!(i2c1.compatible().unwrap(), "st,stm32-i2c-v1");
        assert_eq!(i2c1.address().unwrap(), 0x4000_5400);
        assert_eq!(i2c1.status(), "okay");
        assert!(i2c1.labels.contains(&"arduino_i2c".to_string()));
    }

    #[test]
    fn render_lists_active_nests_children_and_flags_disabled() {
        let map = render(SAMPLE).unwrap();
        assert!(map.contains("Demo Board"));
        assert!(map.contains("arduino_i2c"));
        assert!(map.contains("bosch,bme280")); // sensor on the bus
        assert!(map.contains("@0x40005400"));
        assert!(map.contains("Disabled: i2c2"));
        // The sensor is indented deeper than its bus.
        let bus_line = map.lines().find(|l| l.contains("arduino_i2c")).unwrap();
        let sensor_line = map.lines().find(|l| l.contains("bosch,bme280")).unwrap();
        let indent = |l: &str| l.len() - l.trim_start().len();
        assert!(indent(sensor_line) > indent(bus_line));
    }
}
