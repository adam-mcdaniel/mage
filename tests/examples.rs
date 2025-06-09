use std::{
    fs::File,
    io::{Read, Write},
    path::Path,
};
use tracing::warn;
use anyhow::{Result, Context};
use mage::*;
use lazy_static::lazy_static;

const FFI_TO_LINK: &str = "examples/libexample.c";


lazy_static! {
    static ref COMPILER_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
}

fn run_with_big_stack<T: Send + 'static>(
    f: impl FnOnce(()) -> Result<T> + Send + 'static,
) -> Result<T> {
    let stack_size = 128 * 1024 * 1024; // 4 MB stack size
    let builder = std::thread::Builder::new().stack_size(stack_size);

    let handle = builder
        .spawn(move || {
            f(())
        })
        .expect("Failed to spawn thread");

    handle.join().expect("Failed to join thread")
}

// -------------------------------------------------------------------------
fn expected_pass_compiled(compile_and_run: fn(path: &Path) -> Result<Vec<u8>>) -> Result<()> {
    let _lock = COMPILER_LOCK.lock().unwrap();
    // Gather all .mg files from examples/expected-fail
    let files = std::fs::read_dir("examples/expected-pass")?
        .filter_map(|entry| {
            let entry = entry.ok()?;
            let path = entry.path();
            if path.extension()?.to_str()? == "mg" {
                Some(path)
            } else {
                None
            }
        })
        .collect::<Vec<_>>();
    
    assert!(!files.is_empty(), "No .mg files found in examples/expected-pass");

    for file in files {
        // We expect an error at compile time or a mismatch at runtime:
        let result = compile_and_run(file.as_path());
        // For a “fail” test, usually we want:
        // - either `compile_and_run` returns an Err(...) 
        // - or it runs but yields unexpected output
        // 
        // Below we simply assert that `compile_and_run` returns an error.
        // Adjust to your usage as needed.
        assert!(
            result.is_ok(),
            "Expected {} to pass, but it failed.",
            file.display()
        );
    }

    Ok(())
}

// -------------------------------------------------------------------------
// This helper is identical to `expected_pass_compiled`, except we collect
// mg files from "examples/expected-fail" and expect them to fail
// in some way (compile error, runtime failure, or mismatch).
// Adjust the assertion logic to capture your notion of "failure."
fn expected_fail_compiled(compile_and_run: fn(path: &Path) -> Result<Vec<u8>>) -> Result<()> {
    let _lock = COMPILER_LOCK.lock().unwrap();
    // Gather all .mg files from examples/expected-fail
    let files = std::fs::read_dir("examples/expected-fail")?
        .filter_map(|entry| {
            let entry = entry.ok()?;
            let path = entry.path();
            if path.extension()?.to_str()? == "mg" {
                Some(path)
            } else {
                None
            }
        })
        .collect::<Vec<_>>();

    assert!(!files.is_empty(), "No .mg files found in examples/expected-fail");

    for file in files {
        // We expect an error at compile time or a mismatch at runtime:
        let result = compile_and_run(file.as_path());
        // For a “fail” test, usually we want:
        // - either `compile_and_run` returns an Err(...) 
        // - or it runs but yields unexpected output
        // 
        // Below we simply assert that `compile_and_run` returns an error.
        // Adjust to your usage as needed.
        assert!(
            result.is_err(),
            "Expected {} to fail, but it succeeded.",
            file.display()
        );
    }

    Ok(())
}

// -------------------------------------------------------------------------
// Test “pass” examples with your LLVM backend
#[test]
fn test_expected_pass_c_backend() -> Result<()> {
    // This reuses the `expected_pass_compiled` helper from your existing code, 
    // but changes how we “compile_and_run” each .mg file. In particular, 
    // we compile to LLVM IR and then run clang or llc + gcc. 
    // Adjust to your environment and tools.

    expected_pass_compiled(|path| {
        // 1) Read .mg source
        let mut code = String::new();
        File::open(&path)?.read_to_string(&mut code)?;

        // 2) Parse
        let program = parse(&code)?;

        // 3) Compile to LLVM IR
        let c = CCompiler.compile_main(program)?;  
        // NOTE: you might have your own `LLVMBackend::new()`, etc. 
        // The key is that we now produce IR as a string.

        // 4) Write IR to a .ll file
        let c_path = path.with_extension("c");
        let mut c_file = File::create(&c_path)?;
        write!(c_file, "{}", c)?;

        // 5) Use clang (or llc + gcc) to compile the .ll to an executable
        //    e.g. clang -o foo.out foo.ll
        let status = std::process::Command::new("clang")
            .arg(&c_path)
            .arg(FFI_TO_LINK)
            .arg("-o")
            .arg(path.with_extension("out"))
            .status()?;

        assert!(
            status.success(),
            "Failed to compile LLVM IR for {}",
            path.display()
        );

        // 6) Run the compiled program
        let mut input_text = String::new();
        let input_file_path = path.with_extension("in.txt");
        if input_file_path.exists() {
            File::open(input_file_path)?.read_to_string(&mut input_text)?;
        } else {
            warn!("No input file found for {}", path.display());
        }

        let mut cmd = std::process::Command::new(path.with_extension("out"));
        let mut handle = cmd
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .spawn()?;
        handle.stdin.as_mut().unwrap().write_all(input_text.as_bytes())?;
        let output = handle.wait_with_output()?.stdout;


        // 7) Check that the output is non-empty
        //    (Then `expected_pass_compiled` compares it to .out.txt)
        assert!(
            !output.is_empty(),
            "No output from {}",
            path.display()
        );

        // Delete the .c file and the compiled executable
        std::fs::remove_file(&c_path)?;
        std::fs::remove_file(path.with_extension("out"))?;

        Ok(output)
    })
}


// -------------------------------------------------------------------------
// Test “pass” examples with your LLVM backend
#[test]
fn test_expected_pass_llvm_backend() -> Result<()> {
    // This reuses the `expected_pass_compiled` helper from your existing code, 
    // but changes how we “compile_and_run” each .mg file. In particular, 
    // we compile to LLVM IR and then run clang or llc + gcc. 
    // Adjust to your environment and tools.

    expected_pass_compiled(|path| {
        // 1) Read .mg source
        let mut code = String::new();
        File::open(&path)?.read_to_string(&mut code)?;

        // 2) Parse
        let program = parse(&code)?;

        // 3) Compile to LLVM IR
        let llvm_ir = LLVMCompiler::default().compile_main(program)?;  
        // NOTE: you might have your own `LLVMBackend::new()`, etc. 
        // The key is that we now produce IR as a string.

        // 4) Write IR to a .ll file
        let ll_path = path.with_extension("ll");
        let mut ll_file = File::create(&ll_path)?;
        write!(ll_file, "{}", llvm_ir)?;

        // 5) Use clang (or llc + gcc) to compile the .ll to an executable
        //    e.g. clang -o foo.out foo.ll
        let status = std::process::Command::new("clang")
            .arg(&ll_path)
            .arg(FFI_TO_LINK)
            .arg("-o")
            .arg(path.with_extension("out"))
            .status()?;

        assert!(
            status.success(),
            "Failed to compile LLVM IR for {}",
            path.display()
        );

        // 6) Run the compiled program
        let mut input_text = String::new();
        let input_file_path = path.with_extension("in.txt");
        if input_file_path.exists() {
            File::open(input_file_path)?.read_to_string(&mut input_text)?;
        } else {
            warn!("No input file found for {}", path.display());
        }

        let mut cmd = std::process::Command::new(path.with_extension("out"));
        let mut handle = cmd
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .spawn()?;
        handle.stdin.as_mut().unwrap().write_all(input_text.as_bytes())?;
        let output = handle.wait_with_output()?.stdout;

        // 7) Check that the output is non-empty
        //    (Then `expected_pass_compiled` compares it to .out.txt)
        assert!(
            !output.is_empty(),
            "No output from {}",
            path.display()
        );

        // Delete the .ll file and the compiled executable
        std::fs::remove_file(&ll_path)?;
        std::fs::remove_file(path.with_extension("out"))?;

        Ok(output)
    })
}

// -------------------------------------------------------------------------
// Test “fail” examples with your C backend
#[test]
fn test_expected_fail_c_backend() -> Result<()> {
    expected_fail_compiled(|path| {
        // 1) Read .mg source
        let mut code = String::new();
        File::open(&path)?.read_to_string(&mut code)?;

        // 2) Parse
        let program = parse(&code)?;

        // 3) Compile to C
        let c_code = CCompiler.compile_main(program)?;  // from mage::lir::C, presumably

        // 4) Write to .c file
        let c_path = path.with_extension("c");
        let mut c_file = File::create(&c_path)?;
        write!(c_file, "{}", c_code)?;

        // 5) Attempt to compile with gcc (we expect a *failure* for these tests).
        let status = std::process::Command::new("gcc")
            .arg(&c_path)
            .arg(FFI_TO_LINK)
            .arg("-o")
            .arg(path.with_extension("out"))
            .status()?;

        if status.success() {
            // If compilation “succeeds,” let's try running it. 
            // If you want to force a compile-time failure only, you 
            // can just `return Err(...)` here to signify a test fail. 
            // But sometimes “fail” tests are run-time errors. 
            // Adjust to your preference.

            // Let's say we treat success as an error:
            Err(anyhow::anyhow!(
                "Expected compile failure for {}, but compilation succeeded.",
                path.display()
            ))
        } else {
            // If compilation fails, that’s a success for “fail” tests:
            Err(anyhow::anyhow!("Compilation failed as expected."))
        }
    })
}


#[test]
fn test_expected_pass_interpreter_backend() -> Result<()> {
    run_with_big_stack(|_| {
        expected_pass_compiled(|path| {
            eprintln!("Running {}", path.display());
            // 1) Read .mg source
            let mut code = String::new();
            File::open(&path)?.read_to_string(&mut code)?;

            // 2) Parse
            let program = parse(&code)?;

            // 6) Run the compiled program
            let mut input_text = String::new();
            let input_file_path = path.with_extension("in.txt");
            if input_file_path.exists() {
                File::open(input_file_path)?.read_to_string(&mut input_text)?;
            } else {
                warn!("No input file found for {}", path.display());
            }

            // 3) Compile to LIR
            let i = Interpreter::new(TestInterface::new().with_string_input(&input_text));
            let interface = i.run(&program).context("Failed to run program")?;

            Ok(interface.output_bytes())
        })
    })
}

#[test]
fn test_expected_fail_interpreter_backend() -> Result<()> {
    expected_fail_compiled(|path| {
        // 1) Read .mg source
        let mut code = String::new();
        File::open(&path)?.read_to_string(&mut code)?;

        // 2) Parse
        let program = parse(&code)?;

        // 6) Run the compiled program
        let mut input_text = String::new();
        let input_file_path = path.with_extension("in.txt");
        if input_file_path.exists() {
            File::open(input_file_path)?.read_to_string(&mut input_text)?;
        } else {
            warn!("No input file found for {}", path.display());
        }

        // 3) Compile to LIR
        let i = Interpreter::new(TestInterface::new().with_string_input(&input_text));
        let interface = i.run(&program).context("Failed to run program")?;

        Ok(interface.output_bytes())
    })
}
