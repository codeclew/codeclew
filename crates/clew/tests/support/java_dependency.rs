//! Synthetic dependency fixture using ordinary JAR bytes, never product-private state.
use clew::canonical;
use clew::java_adapter_v2::{JavaCompilerIndex, build_java_compiler_index};
use clew::java_project_model::{
    JavaBuildSystem, JavaClasspathAuthority, JavaOperationalModel, JavaProjectModel,
};
use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::process::Command;
use zip::write::{SimpleFileOptions, ZipWriter};

pub const OWNER: &str = "class:example.integration.InventoryGateway";
pub const TARGET: &str =
    "method:class:example.integration.InventoryGateway#reserve(Ljava/lang/String;I)Z";
pub const SERVICE_FILE: &str = "src/example/Service.java";
pub const SERVICE_SOURCE: &str = "package example;\nimport example.integration.InventoryGateway;\npublic class Service {\n private final InventoryGateway gateway;\n public Service(InventoryGateway gateway) { this.gateway = gateway; }\n public boolean reserve(String sku, int quantity) { return gateway.reserve(sku, quantity); }\n}\n";

fn tool(name: &str) -> PathBuf {
    std::env::var_os("JAVA_HOME")
        .map(|home| PathBuf::from(home).join("bin").join(name))
        .unwrap_or_else(|| name.into())
}
fn run(command: &mut Command) -> std::process::Output {
    let output = command.output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

pub struct Fixture {
    pub temporary: tempfile::TempDir,
    pub model: JavaOperationalModel,
    pub source_digests: BTreeMap<String, String>,
}
impl Fixture {
    pub fn new() -> Self {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path();
        let dependency = root.join("dependency-src/example/integration/InventoryGateway.java");
        fs::create_dir_all(dependency.parent().unwrap()).unwrap();
        fs::write(&dependency, "package example.integration; public interface InventoryGateway { boolean reserve(String sku, int quantity); boolean reserve(String sku); }").unwrap();
        let classes = root.join("dependency-classes");
        fs::create_dir(&classes).unwrap();
        run(Command::new(tool("javac"))
            .args(["--release", "17", "-d"])
            .arg(&classes)
            .arg(dependency));
        let jar = root.join("deliberately-unversioned.jar");
        let mut zip = ZipWriter::new(fs::File::create(&jar).unwrap());
        let options = SimpleFileOptions::default();
        for (name, bytes) in [
            ("META-INF/MANIFEST.MF", b"Manifest-Version: 1.0\r\nAutomatic-Module-Name: example.inventory\r\nImplementation-Version: 2.3.4\r\n\r\n".to_vec()),
            ("META-INF/maven/example/inventory/pom.properties", b"groupId=example\nartifactId=inventory\nversion=2.3.4\n".to_vec()),
            ("example/integration/InventoryGateway.class", fs::read(classes.join("example/integration/InventoryGateway.class")).unwrap()),
        ] {
            zip.start_file(name, options).unwrap();
            zip.write_all(&bytes).unwrap();
        }
        zip.finish().unwrap();
        let bytes = fs::read(&jar).unwrap();
        let digest = canonical::hash_bytes(&bytes);
        let source = root.join(SERVICE_FILE);
        fs::create_dir_all(source.parent().unwrap()).unwrap();
        fs::write(&source, SERVICE_SOURCE).unwrap();
        let compiler = run(Command::new(tool("javac")).arg("--version"));
        let mut authority = JavaProjectModel {
            schema: clew::java_project_model::JAVA_MODEL_SCHEMA.into(),
            model_digest: String::new(),
            build_system: JavaBuildSystem::Maven,
            compilation: ":/main".into(),
            source_files: vec![SERVICE_FILE.into()],
            classpath: vec![JavaClasspathAuthority {
                logical_name: format!(
                    "artifact:deliberately-unversioned.jar:{}",
                    digest.trim_start_matches("sha256:")
                ),
                digest,
                size: bytes.len() as u64,
                kind: "FILE".into(),
            }],
            dependency_sources: vec![],
            release: 17,
            compiler_version: String::from_utf8(compiler.stdout).unwrap().trim().into(),
            compiler_options: vec!["--release=17".into(), "-implicit:none".into()],
            annotation_processors: vec![],
            annotation_processor_paths: vec![],
            boundaries: vec![],
        };
        authority.model_digest = canonical::hash(&authority).unwrap();
        let model = JavaOperationalModel {
            authority,
            source_paths: vec![source],
            classpath_paths: vec![jar],
            annotation_processor_paths: vec![],
            java_executable: tool("java"),
        };
        Self {
            temporary,
            model,
            source_digests: BTreeMap::from([(
                SERVICE_FILE.into(),
                canonical::hash_bytes(SERVICE_SOURCE.as_bytes()),
            )]),
        }
    }
    pub fn index(&self) -> JavaCompilerIndex {
        build_java_compiler_index(
            self.temporary.path(),
            &self.model,
            &self.source_digests,
            false,
            None,
            &[],
            None,
        )
        .unwrap()
    }
}

impl Fixture {
    pub fn set_source(&mut self, source: &str) {
        fs::write(self.temporary.path().join(SERVICE_FILE), source).unwrap();
        self.source_digests.insert(
            SERVICE_FILE.into(),
            canonical::hash_bytes(source.as_bytes()),
        );
    }
    pub fn attach_sources(&mut self, source: &str) -> PathBuf {
        self.attach_source_entries(&[("example/integration/InventoryGateway.java", source)])
    }
    pub fn attach_source_entries(&mut self, entries: &[(&str, &str)]) -> PathBuf {
        let archive = self.temporary.path().join("inventory-2.3.4-sources.jar");
        let mut zip = ZipWriter::new(fs::File::create(&archive).unwrap());
        for (entry, source) in entries {
            zip.start_file(*entry, SimpleFileOptions::default())
                .unwrap();
            zip.write_all(source.as_bytes()).unwrap();
        }
        zip.finish().unwrap();
        let bytes = fs::read(&archive).unwrap();
        let digest = canonical::hash_bytes(&bytes);
        self.model.authority.dependency_sources =
            vec![clew::java_project_model::JavaDependencySourceAuthority {
                binary_digest: self.model.authority.classpath[0].digest.clone(),
                coordinate: "example:inventory:2.3.4".into(),
                source_archive: JavaClasspathAuthority {
                    logical_name: format!(
                        "artifact:inventory-2.3.4-sources.jar:{}",
                        digest.trim_start_matches("sha256:")
                    ),
                    digest,
                    size: bytes.len() as u64,
                    kind: "FILE".into(),
                },
            }];
        self.rehash_model();
        archive
    }
    pub fn rehash_model(&mut self) {
        self.model.authority.model_digest.clear();
        self.model.authority.model_digest = canonical::hash(&self.model.authority).unwrap();
    }
    pub fn prepend_shadow_jar(&mut self) {
        let archive = self.temporary.path().join("shadow.jar");
        let mut zip = ZipWriter::new(fs::File::create(&archive).unwrap());
        for (entry, bytes) in [
            (
                "META-INF/maven/example/inventory/pom.properties",
                b"groupId=example\nartifactId=inventory\nversion=9.9.9\n".to_vec(),
            ),
            (
                "example/integration/InventoryGateway.class",
                fs::read(
                    self.temporary
                        .path()
                        .join("dependency-classes/example/integration/InventoryGateway.class"),
                )
                .unwrap(),
            ),
        ] {
            zip.start_file(entry, SimpleFileOptions::default()).unwrap();
            zip.write_all(&bytes).unwrap();
        }
        zip.finish().unwrap();
        let bytes = fs::read(&archive).unwrap();
        let digest = canonical::hash_bytes(&bytes);
        self.model.authority.classpath.insert(
            0,
            JavaClasspathAuthority {
                logical_name: format!(
                    "artifact:shadow.jar:{}",
                    digest.trim_start_matches("sha256:")
                ),
                digest,
                size: bytes.len() as u64,
                kind: "FILE".into(),
            },
        );
        self.model.classpath_paths.insert(0, archive);
        self.rehash_model();
    }
}

impl Fixture {
    pub fn use_class_directory(&mut self) {
        let directory = self.temporary.path().join("dependency-classes");
        let relative = "example/integration/InventoryGateway.class";
        let bytes = fs::read(directory.join(relative)).unwrap();
        let entries = vec![(
            relative.to_string(),
            canonical::hash_bytes(&bytes),
            bytes.len() as u64,
        )];
        let digest = canonical::hash(&entries).unwrap();
        self.model.authority.classpath = vec![JavaClasspathAuthority {
            logical_name: format!("directory:{}", digest.trim_start_matches("sha256:")),
            digest,
            size: bytes.len() as u64,
            kind: "DIRECTORY".into(),
        }];
        self.model.classpath_paths = vec![directory];
        self.rehash_model();
    }
}

fn directory_authority(directory: &std::path::Path) -> JavaClasspathAuthority {
    let mut entries = Vec::new();
    for entry in walkdir::WalkDir::new(directory) {
        let entry = entry.unwrap();
        if entry.file_type().is_file() {
            let bytes = fs::read(entry.path()).unwrap();
            let relative = entry
                .path()
                .strip_prefix(directory)
                .unwrap()
                .to_str()
                .unwrap()
                .replace('\\', "/");
            entries.push((relative, canonical::hash_bytes(&bytes), bytes.len() as u64));
        }
    }
    entries.sort();
    let digest = canonical::hash(&entries).unwrap();
    JavaClasspathAuthority {
        logical_name: format!("directory:{}", digest.trim_start_matches("sha256:")),
        digest,
        size: entries.iter().map(|entry| entry.2).sum(),
        kind: "DIRECTORY".into(),
    }
}

impl Fixture {
    pub fn replace_dependencies(&mut self, sources: &[(&str, &str)]) -> PathBuf {
        let classes = self.temporary.path().join("replacement-classes");
        fs::create_dir(&classes).unwrap();
        let mut javac = Command::new(tool("javac"));
        javac.args(["--release", "17", "-d"]).arg(&classes);
        for (entry, source) in sources {
            let path = self.temporary.path().join("replacement-src").join(entry);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, source).unwrap();
            javac.arg(path);
        }
        run(&mut javac);
        let jar = &self.model.classpath_paths[0];
        let mut zip = ZipWriter::new(fs::File::create(jar).unwrap());
        zip.start_file(
            "META-INF/maven/example/inventory/pom.properties",
            SimpleFileOptions::default(),
        )
        .unwrap();
        zip.write_all(b"groupId=example\nartifactId=inventory\nversion=2.3.4\n")
            .unwrap();
        for entry in walkdir::WalkDir::new(&classes) {
            let entry = entry.unwrap();
            if entry.file_type().is_file() {
                let relative = entry
                    .path()
                    .strip_prefix(&classes)
                    .unwrap()
                    .to_str()
                    .unwrap()
                    .replace('\\', "/");
                zip.start_file(relative, SimpleFileOptions::default())
                    .unwrap();
                zip.write_all(&fs::read(entry.path()).unwrap()).unwrap();
            }
        }
        zip.finish().unwrap();
        let bytes = fs::read(jar).unwrap();
        let digest = canonical::hash_bytes(&bytes);
        self.model.authority.classpath[0] = JavaClasspathAuthority {
            logical_name: format!(
                "artifact:deliberately-unversioned.jar:{}",
                digest.trim_start_matches("sha256:")
            ),
            digest,
            size: bytes.len() as u64,
            kind: "FILE".into(),
        };
        self.model.authority.dependency_sources.clear();
        self.rehash_model();
        classes
    }
    pub fn set_classpath_directories(&mut self, directories: Vec<PathBuf>) {
        self.model.authority.classpath = directories
            .iter()
            .map(|path| directory_authority(path))
            .collect();
        self.model.classpath_paths = directories;
        self.model.authority.dependency_sources.clear();
        self.rehash_model();
    }
}
