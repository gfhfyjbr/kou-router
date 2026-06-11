use std::{
    env, fs, io,
    path::{Path, PathBuf},
    process::Command,
};

fn main() {
    let manifest_dir = PathBuf::from(
        env::var_os("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR must be set by Cargo"),
    );
    let frontend_dir = manifest_dir.join("frontend");
    let dist_dir = frontend_dir.join("dist");

    println!("cargo:rerun-if-env-changed=KOU_SKIP_FRONTEND_BUILD");
    println!("cargo:rerun-if-env-changed=KOU_FRONTEND_PACKAGE_MANAGER");
    for path in [
        "package.json",
        "bun.lock",
        "index.html",
        "vite.config.ts",
        "tsconfig.json",
        "tsconfig.app.json",
        "tsconfig.node.json",
    ] {
        println!(
            "cargo:rerun-if-changed={}",
            frontend_dir.join(path).display()
        );
    }
    print_rerun_if_changed(&frontend_dir.join("src")).expect("scan frontend/src");

    fs::create_dir_all(&dist_dir).expect("create frontend/dist");

    if env_truthy("KOU_SKIP_FRONTEND_BUILD") {
        eprintln!("KOU_SKIP_FRONTEND_BUILD is set; skipping frontend build");
        return;
    }

    if !frontend_dir.join("package.json").exists() {
        eprintln!("frontend/package.json is missing; skipping frontend build");
        return;
    }

    let package_manager =
        env::var("KOU_FRONTEND_PACKAGE_MANAGER").unwrap_or_else(|_| "bun".to_string());
    ensure_frontend_dependencies(&frontend_dir, &package_manager);
    run_frontend_build(&frontend_dir, &package_manager);
}

fn ensure_frontend_dependencies(frontend_dir: &Path, package_manager: &str) {
    if frontend_dir.join("node_modules").exists() {
        return;
    }

    let args: &[&str] = match package_manager {
        "bun" => {
            if frontend_dir.join("bun.lock").exists() {
                &["install", "--frozen-lockfile"]
            } else {
                &["install"]
            }
        }
        "npm" => {
            if frontend_dir.join("package-lock.json").exists() {
                &["ci"]
            } else {
                &["install"]
            }
        }
        "pnpm" => &["install", "--frozen-lockfile"],
        "yarn" => &["install", "--frozen-lockfile"],
        other => panic!("unsupported KOU_FRONTEND_PACKAGE_MANAGER={other}"),
    };

    run_command(
        frontend_dir,
        package_manager,
        args,
        "frontend dependency install",
    );
}

fn run_frontend_build(frontend_dir: &Path, package_manager: &str) {
    let args: &[&str] = match package_manager {
        "bun" => &["run", "build"],
        "npm" => &["run", "build"],
        "pnpm" => &["run", "build"],
        "yarn" => &["build"],
        other => panic!("unsupported KOU_FRONTEND_PACKAGE_MANAGER={other}"),
    };

    run_command(frontend_dir, package_manager, args, "frontend build");
}

fn run_command(frontend_dir: &Path, program: &str, args: &[&str], label: &str) {
    let status = Command::new(program)
        .args(args)
        .current_dir(frontend_dir)
        .status()
        .unwrap_or_else(|err| {
            panic!(
                "failed to run {label} with `{program} {}` in {}: {err}. \
                 Install Bun or set KOU_FRONTEND_PACKAGE_MANAGER=npm/pnpm/yarn.",
                args.join(" "),
                frontend_dir.display()
            )
        });

    if !status.success() {
        panic!(
            "{label} failed with status {status}. \
             Set KOU_SKIP_FRONTEND_BUILD=1 to skip embedding during local experiments."
        );
    }
}

fn print_rerun_if_changed(path: &Path) -> io::Result<()> {
    if !path.exists() {
        return Ok(());
    }

    let mut stack = vec![PathBuf::from(path)];
    while let Some(path) = stack.pop() {
        let metadata = fs::metadata(&path)?;
        if metadata.is_dir() {
            for entry in fs::read_dir(&path)? {
                let entry = entry?;
                let child = entry.path();
                let name = child
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("");
                if name == "node_modules" || name == "dist" {
                    continue;
                }
                stack.push(child);
            }
        } else if metadata.is_file() {
            println!("cargo:rerun-if-changed={}", path.display());
        }
    }
    Ok(())
}

fn env_truthy(name: &str) -> bool {
    env::var(name)
        .map(|value| {
            let value = value.trim().to_ascii_lowercase();
            matches!(value.as_str(), "1" | "true" | "yes" | "on")
        })
        .unwrap_or(false)
}
