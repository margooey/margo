/*
    Maven cargo = margo

    50% vibe-coded because I am NOT spending real hours on this
    I have to learn Java for university and I genuinely believe manually
    editing pomfiles is archaic and stupid

    You get two commands

    margo add
    margo remove

    Example:

    margo add org.apache.commons:commons-lang3@3.12.0
    margo remove org.apache.commons:commons-lang3
*/

use clap::{Parser, Subcommand};
use minidom::{Element, NSChoice};
use std::env;
use std::fs;
use std::io::BufReader;
use xmlformat::Formatter;

const NS: &str = "http://maven.apache.org/POM/4.0.0";

/// Like Cargo but for Maven
#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Args {
    /// Subcommand to run
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand, Debug)]
enum Commands {
    Add {
        /// Dependency to add
        dep: String,
    },
    Remove {
        /// Dependency to remove
        dep: String,
    },
}

fn child_text(el: &Element, name: &str) -> String {
    el.get_child(name, NSChoice::Any)
        .map(|e| e.text())
        .unwrap_or_default()
}

fn find_dependency<'a>(deps: &'a Element, g: &str, a: &str) -> Option<&'a Element> {
    deps.children()
        .find(|d| child_text(d, "groupId") == g && child_text(d, "artifactId") == a)
}

fn build_dependency(g: &str, a: &str, v: &str) -> Element {
    Element::builder("dependency", NS)
        .append(Element::builder("groupId", NS).append(g).build())
        .append(Element::builder("artifactId", NS).append(a).build())
        .append(Element::builder("version", NS).append(v).build())
        .build()
}

fn parse_pom(pom: &str) -> Element {
    Element::from_reader(BufReader::new(pom.as_bytes())).expect("Failed to parse pom.xml")
}

fn serialize_pom(root: &Element) -> String {
    let mut buf = Vec::new();
    root.write_to_decl(&mut buf).unwrap();
    let xml = String::from_utf8(buf).unwrap();
    Formatter {
        compress: false,
        indent: 4,
        keep_comments: true,
        eof_newline: true,
    }
    .format_xml(&xml)
    .unwrap()
}

fn add_dependency_to_pom(pom: &str, g: &str, a: &str, v: &str) -> String {
    let mut root = parse_pom(pom);
    let deps = root.get_child_mut("dependencies", NSChoice::Any);

    match deps {
        Some(deps) => {
            // Remove existing entry if present (overwrite case)
            if find_dependency(deps, g, a).is_some() {
                let nodes = deps.take_nodes();
                for node in nodes {
                    if let minidom::Node::Element(ref e) = node {
                        if child_text(e, "groupId") == g && child_text(e, "artifactId") == a {
                            continue;
                        }
                    }
                    deps.append_node(node);
                }
            }
            deps.append_child(build_dependency(g, a, v));
        }
        None => {
            let mut deps = Element::bare("dependencies", NS);
            deps.append_child(build_dependency(g, a, v));
            root.append_child(deps);
        }
    }

    serialize_pom(&root)
}

fn remove_dependency_from_pom(pom: &str, g: &str, a: &str) -> Result<String, String> {
    let mut root = parse_pom(pom);
    let deps = root
        .get_child_mut("dependencies", NSChoice::Any)
        .ok_or_else(|| format!("{}:{} is not in pom.xml", g, a))?;

    if find_dependency(deps, g, a).is_none() {
        return Err(format!("{}:{} is not in pom.xml", g, a));
    }

    let nodes = deps.take_nodes();
    for node in nodes {
        if let minidom::Node::Element(ref e) = node {
            if child_text(e, "groupId") == g && child_text(e, "artifactId") == a {
                continue;
            }
        }
        deps.append_node(node);
    }

    Ok(serialize_pom(&root))
}

fn main() {
    let args = Args::parse();

    match &args.command {
        Some(Commands::Add { dep }) => {
            let dir = env::current_dir().unwrap();

            // Split dependency into groupId:artifactId[@version]
            let (coords, version) = match dep.split_once('@') {
                Some((c, v)) => (c, Some(v)),
                None => (dep.as_str(), None),
            };
            let (group_id, artifact_id) = coords
                .split_once(':')
                .expect("Dependency must be in the format groupId:artifactId[@version]");

            let pom_path = dir.join("pom.xml");
            assert!(
                pom_path.exists(),
                "No pom.xml found in the current directory."
            );

            // Fetch dependency from Maven Central
            let url = match version {
                Some(v) => format!(
                    "https://search.maven.org/solrsearch/select?q=g:%22{}%22+AND+a:%22{}%22+AND+v:%22{}%22&core=gav",
                    group_id, artifact_id, v
                ),
                None => format!(
                    "https://search.maven.org/solrsearch/select?q=g:%22{}%22+AND+a:%22{}%22&rows=1&core=gav",
                    group_id, artifact_id
                ),
            };
            let body: String = ureq::get(&url)
                .call()
                .unwrap()
                .into_body()
                .read_to_string()
                .unwrap();

            // Parse JSON response
            let json: serde_json::Value =
                serde_json::from_str(&body).expect("Invalid JSON from Maven Central");
            let doc = &json["response"]["docs"][0];
            let doc = match doc.as_object() {
                Some(d) => d,
                None => {
                    eprintln!(
                        "\x1b[1;31merror\x1b[0m: Dependency or version not found on Maven Central: {}:{}@{}",
                        group_id, artifact_id, version.unwrap_or("latest")
                    );
                    std::process::exit(1);
                }
            };
            let g = doc["g"].as_str().unwrap();
            let a = doc["a"].as_str().unwrap();
            let v = doc["v"].as_str().unwrap();

            let pom = fs::read_to_string(&pom_path).unwrap();
            let updated = add_dependency_to_pom(&pom, g, a, v);
            fs::write(&pom_path, updated).unwrap();

            println!("Added {}:{}:{} to pom.xml", g, a, v);
        }
        Some(Commands::Remove { dep }) => {
            let dir = env::current_dir().unwrap();
            let (group_id, artifact_id) = dep
                .split_once(':')
                .expect("Dependency must be in the format groupId:artifactId");

            let pom_path = dir.join("pom.xml");
            assert!(
                pom_path.exists(),
                "No pom.xml found in the current directory."
            );

            let pom = fs::read_to_string(&pom_path).unwrap();
            let updated =
                remove_dependency_from_pom(&pom, group_id, artifact_id).unwrap_or_else(|e| {
                    eprintln!("\x1b[1;31merror\x1b[0m: {}", e);
                    std::process::exit(1);
                });
            fs::write(&pom_path, updated).unwrap();

            println!("Removed {}:{} from pom.xml", group_id, artifact_id);
        }
        None => {
            println!("No subcommand")
        }
    }
}
