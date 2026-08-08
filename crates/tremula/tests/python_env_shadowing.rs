//! One test, alone in its own binary because it changes the working directory.
//!
//! A directory named `tremula_python` in the working directory is an ordinary
//! thing for a checkout of the pack itself to have, and Python would import it in
//! preference to the installed distribution. Verifying the pack has to be immune
//! to that, or the core would go on to run a pack that is not the one it
//! negotiated with. The working directory is process-wide state, so this case
//! lives in a file of its own rather than beside tests that share a process.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::{env, fs, path::Path};

use tempfile::TempDir;
use tremula::python_env::PythonEnv;

#[test]
fn a_decoy_package_in_the_working_directory_is_not_mistaken_for_the_pack() {
    let interpreter = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(".venv")
        .join("bin")
        .join("python");
    if !interpreter.is_file() {
        eprintln!(
            "skipped: no virtual environment at {}",
            interpreter.display()
        );
        return;
    }
    let decoy = TempDir::new().unwrap();
    let package = decoy.path().join("tremula_python");
    fs::create_dir_all(&package).unwrap();
    fs::write(
        package.join("__init__.py"),
        "raise RuntimeError('the decoy was imported')\n",
    )
    .unwrap();
    env::set_current_dir(decoy.path()).unwrap();

    let pack = PythonEnv::at(&interpreter).verify_pack().unwrap();

    assert_eq!(pack.distribution, "tremula-python");
}
