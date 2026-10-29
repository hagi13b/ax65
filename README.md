# AX65 repository

This repository holds the AX65 compiler, specification, reference manual, sample programs, and historical materials. The project layout is intentionally flat: versioning is handled by Git tags, not by folder names such as `ax65_1.1/`.

## Directory layout

- `compiler/` : current compiler source and build tools
- `spec/` : language specification documents
- `manual/` : user manual and examples
- `sample/` : sample programs and fixtures
- `editors/` : editor syntax/highlighting support
- `archive/` : historical builds, drafts, notes, and older snapshots
- `scripts/` : release and maintenance helper scripts
- `.vscode/` : workspace settings and tasks

## Git policy

- Source materials are tracked in Git.
- Build artifacts such as `target/`, `build/`, generated `.bin`, `.lst`, `.sym`, `.report`, and temporary files are ignored.
- `archive/` is kept as historical reference material, but binary-heavy generated files are excluded from release zip packages unless explicitly requested.
- Release versioning is managed by Git tags, not by directory names.

## Typical commands

From the repository root:

```bash
# Build the compiler
cd compiler
cargo build --release

# Run the compiler
cargo run -- --help

# Build all sample projects
./scripts/build_samples.sh

# Build all example projects
./scripts/build_examples.sh

# Create a clean release zip without archive/
./scripts/package_release.sh

# Create a release zip including archive/
./scripts/package_release.sh --with-archive
```

The packaging script writes files under `dist/` by default and strips common build artifacts, object files, logs, and generated output.

## VS Code tasks

The workspace includes tasks for:

- building the compiler
- running the compiler
- building all sample projects
- building all example projects
- creating a clean release zip
- checking Git status

Open the VS Code task list and run the task you need.
