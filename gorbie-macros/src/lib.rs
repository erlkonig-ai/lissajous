use proc_macro::TokenStream;
use proc_macro2::Span;
use proc_macro_crate::{crate_name, FoundCrate};
use quote::quote;
use syn::parse::{Parse, ParseStream};
use syn::{parse_macro_input, Ident, ItemFn, LitStr, Result, Token};

struct NotebookAttr {
    name: Option<LitStr>,
}

impl Parse for NotebookAttr {
    fn parse(input: ParseStream) -> Result<Self> {
        if input.is_empty() {
            return Ok(Self { name: None });
        }

        let name_key: Ident = input.parse()?;
        if name_key != "name" {
            return Err(input.error("expected `name = \"...\"`"));
        }
        input.parse::<Token![=]>()?;
        let name: LitStr = input.parse()?;
        if input.peek(Token![,]) {
            input.parse::<Token![,]>()?;
        }
        if !input.is_empty() {
            return Err(input.error("unexpected tokens"));
        }

        Ok(Self { name: Some(name) })
    }
}

#[proc_macro_attribute]
pub fn notebook(attr: TokenStream, item: TokenStream) -> TokenStream {
    let NotebookAttr { name } = parse_macro_input!(attr as NotebookAttr);
    let mut input = parse_macro_input!(item as ItemFn);
    let gorbie = gorbie_path();
    let original_ident = input.sig.ident.clone();
    let body_ident = Ident::new(
        &format!("__gorbie_{}_body", original_ident),
        Span::call_site(),
    );
    input.sig.ident = body_ident.clone();
    let vis = input.vis.clone();

    let mut setup_stmts: Vec<syn::Stmt> = Vec::new();
    if let Some(name) = name {
        setup_stmts.push(syn::parse_quote!(
            let mut __gorbie_notebook_owner = #gorbie::NotebookConfig::new(#name);
        ));
    } else {
        setup_stmts.push(syn::parse_quote!(let __gorbie_notebook_file = file!();));
        setup_stmts.push(syn::parse_quote!(
            let __gorbie_notebook_name = std::path::Path::new(__gorbie_notebook_file)
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or(__gorbie_notebook_file);
        ));
        setup_stmts.push(syn::parse_quote!(
            let mut __gorbie_notebook_owner =
                #gorbie::NotebookConfig::new(__gorbie_notebook_name);
        ));
    }

    let wrapper = quote! {
        #vis fn #original_ident() {
            #(#setup_stmts)*

            #[cfg(not(target_arch = "wasm32"))]
            {
                let mut __gorbie_headless = false;
                let mut __gorbie_headless_out_dir: Option<std::path::PathBuf> = None;
                let mut __gorbie_headless_scale: Option<f32> = None;
                let mut __gorbie_headless_wait_ms: Option<u64> = None;
                let mut __gorbie_headless_theme: Option<#gorbie::HeadlessTheme> = None;
                let mut __gorbie_export = false;
                let mut __gorbie_export_out_dir: Option<std::path::PathBuf> = None;
                let mut __gorbie_args = std::env::args().skip(1);
                while let Some(arg) = __gorbie_args.next() {
                    match arg.as_str() {
                        "--headless" => {
                            __gorbie_headless = true;
                        }
                        "--out-dir" => {
                            if let Some(dir) = __gorbie_args.next() {
                                __gorbie_headless_out_dir = Some(std::path::PathBuf::from(dir));
                            } else {
                                eprintln!("--out-dir expects a path");
                                return;
                            }
                        }
                        "--scale" | "--pixels-per-point" => {
                            if let Some(scale) = __gorbie_args.next() {
                                match scale.parse::<f32>() {
                                    Ok(value) if value > 0.0 => {
                                        __gorbie_headless_scale = Some(value);
                                    }
                                    _ => {
                                        eprintln!("--scale expects a positive number");
                                        return;
                                    }
                                }
                            } else {
                                eprintln!("--scale expects a number");
                                return;
                            }
                        }
                        "--headless-wait-ms" => {
                            if let Some(wait_ms) = __gorbie_args.next() {
                                match wait_ms.parse::<u64>() {
                                    Ok(value) => {
                                        __gorbie_headless_wait_ms = Some(value);
                                    }
                                    _ => {
                                        eprintln!("--headless-wait-ms expects a non-negative integer");
                                        return;
                                    }
                                }
                            } else {
                                eprintln!("--headless-wait-ms expects a number");
                                return;
                            }
                        }
                        "--theme" => {
                            match __gorbie_args.next().as_deref() {
                                Some("light") => {
                                    __gorbie_headless_theme = Some(#gorbie::HeadlessTheme::Light);
                                }
                                Some("dark") => {
                                    __gorbie_headless_theme = Some(#gorbie::HeadlessTheme::Dark);
                                }
                                Some("auto") => {
                                    __gorbie_headless_theme = Some(#gorbie::HeadlessTheme::Auto);
                                }
                                _ => {
                                    eprintln!("--theme expects light, dark, or auto");
                                    return;
                                }
                            }
                        }
                        "--light" => {
                            __gorbie_headless_theme = Some(#gorbie::HeadlessTheme::Light);
                        }
                        "--dark" => {
                            __gorbie_headless_theme = Some(#gorbie::HeadlessTheme::Dark);
                        }
                        "--export" => {
                            __gorbie_export = true;
                        }
                        "--export-dir" => {
                            if let Some(dir) = __gorbie_args.next() {
                                __gorbie_export_out_dir = Some(std::path::PathBuf::from(dir));
                            } else {
                                eprintln!("--export-dir expects a path");
                                return;
                            }
                        }
                        _ => {}
                    }
                }

                if __gorbie_export {
                    #[cfg(feature = "web-export")]
                    {
                        #gorbie::__gorbie_web_export!(__gorbie_export_out_dir);
                    }
                    #[cfg(not(feature = "web-export"))]
                    {
                        eprintln!("web export not available in this build.");
                        eprintln!("rebuild with:");
                        eprintln!("  cargo run --features web-export --release -- --export");
                    }
                    return;
                }

                if __gorbie_headless {
                    if let Some(theme) = __gorbie_headless_theme {
                        __gorbie_notebook_owner =
                            __gorbie_notebook_owner.with_headless_theme(theme);
                    }
                    let out_dir = __gorbie_headless_out_dir
                        .unwrap_or_else(|| std::path::PathBuf::from("gorbie_capture"));
                    __gorbie_notebook_owner = if let Some(scale) = __gorbie_headless_scale {
                        __gorbie_notebook_owner.with_headless_capture_scaled(out_dir, scale)
                    } else {
                        __gorbie_notebook_owner.with_headless_capture(out_dir)
                    };
                    if let Some(wait_ms) = __gorbie_headless_wait_ms {
                        __gorbie_notebook_owner = __gorbie_notebook_owner
                            .with_headless_settle_timeout(std::time::Duration::from_millis(wait_ms));
                    }
                }
            }

            __gorbie_notebook_owner
                .run(|__gorbie_notebook_ctx| {
                    #body_ident(__gorbie_notebook_ctx);
                })
                .unwrap();
        }
    };

    TokenStream::from(quote! {
        #input
        #wrapper
    })
}

#[proc_macro]
pub fn __gorbie_web_export(input: TokenStream) -> TokenStream {
    let out_dir_ident = parse_macro_input!(input as Ident);
    let bin_name = std::env::var("CARGO_BIN_NAME")
        .or_else(|_| std::env::var("CARGO_PKG_NAME"))
        .unwrap_or_else(|_| "notebook".to_owned());
    let crate_name = bin_name.replace('-', "_");
    TokenStream::from(build_wasm_export(&bin_name, &crate_name, &out_dir_ident))
}

fn build_wasm_export(
    bin_name: &str,
    crate_name: &str,
    out_dir_ident: &Ident,
) -> proc_macro2::TokenStream {
    let manifest_dir = match std::env::var("CARGO_MANIFEST_DIR") {
        Ok(d) => d,
        Err(_) => {
            return quote! { compile_error!("CARGO_MANIFEST_DIR not set"); };
        }
    };

    let target_dir = format!("/tmp/gorbie-web-export/{crate_name}");
    let dist_dir = format!("{target_dir}/dist");

    eprintln!("gorbie: building wasm for web export...");

    let cargo_status = std::process::Command::new("cargo")
        .args([
            "build",
            "--target",
            "wasm32-unknown-unknown",
            "--release",
            "--frozen",
            "--target-dir",
            &target_dir,
            "--manifest-path",
            &format!("{manifest_dir}/Cargo.toml"),
            "--bin",
            bin_name,
        ])
        .env_remove("GORBIE_WEB_EXPORT")
        .status();

    match cargo_status {
        Ok(s) if !s.success() => {
            return quote! { compile_error!("gorbie web export: wasm build failed"); };
        }
        Err(e) => {
            let msg = format!("gorbie web export: failed to run cargo: {e}");
            return quote! { compile_error!(#msg); };
        }
        _ => {}
    }

    let wasm_bindgen = match find_wasm_bindgen() {
        Some(path) => path,
        None => {
            return quote! {
                compile_error!(
                    "gorbie web export: wasm-bindgen not found. \
                     Install with: cargo install wasm-bindgen-cli"
                );
            };
        }
    };

    let wasm_input = format!("{target_dir}/wasm32-unknown-unknown/release/{bin_name}.wasm");

    if !std::path::Path::new(&wasm_input).exists() {
        let msg = format!("gorbie web export: wasm file not found at {wasm_input}");
        return quote! { compile_error!(#msg); };
    }

    let _ = std::fs::create_dir_all(&dist_dir);

    let bindgen_status = std::process::Command::new(&wasm_bindgen)
        .args(["--out-dir", &dist_dir, "--target", "web", "--no-typescript"])
        .arg(&wasm_input)
        .status();

    match bindgen_status {
        Ok(s) if !s.success() => {
            return quote! { compile_error!("gorbie web export: wasm-bindgen failed"); };
        }
        Err(e) => {
            let msg = format!("gorbie web export: failed to run wasm-bindgen: {e}");
            return quote! { compile_error!(#msg); };
        }
        _ => {}
    }

    let wasm_path = format!("{dist_dir}/{bin_name}_bg.wasm");
    let js_path = format!("{dist_dir}/{bin_name}.js");

    if !std::path::Path::new(&wasm_path).exists() {
        let msg = format!("gorbie web export: expected {wasm_path}");
        return quote! { compile_error!(#msg); };
    }
    if !std::path::Path::new(&js_path).exists() {
        let msg = format!("gorbie web export: expected {js_path}");
        return quote! { compile_error!(#msg); };
    }

    let wasm_size = std::fs::metadata(&wasm_path).map(|m| m.len()).unwrap_or(0);
    eprintln!(
        "gorbie: web export built ({:.1} MB wasm)",
        wasm_size as f64 / (1024.0 * 1024.0)
    );

    let wasm_path_lit = LitStr::new(&wasm_path, Span::call_site());
    let js_path_lit = LitStr::new(&js_path, Span::call_site());
    let wasm_filename = format!("{bin_name}_bg.wasm");
    let js_filename = format!("{bin_name}.js");

    let html_template = format!(
        r##"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>{bin_name}</title>
<style>
html,body{{margin:0;padding:0;width:100%;height:100%;overflow:hidden;background:#cfd0cf}}
@media(prefers-color-scheme:dark){{html,body{{background:#82898e}}}}
canvas{{display:block;width:100%;height:100%}}
</style>
</head>
<body>
<canvas id="gorbie_canvas"></canvas>
<script type="module">
import init from './{js_filename}';
await init('./{wasm_filename}');
</script>
</body>
</html>"##
    );

    quote! {
        let __gorbie_out = #out_dir_ident
            .unwrap_or_else(|| std::path::PathBuf::from("gorbie_export"));
        std::fs::create_dir_all(&__gorbie_out)
            .expect("failed to create export directory");
        std::fs::write(
            __gorbie_out.join(#wasm_filename),
            include_bytes!(#wasm_path_lit),
        ).expect("failed to write wasm");
        std::fs::write(
            __gorbie_out.join(#js_filename),
            include_bytes!(#js_path_lit),
        ).expect("failed to write js");
        std::fs::write(
            __gorbie_out.join("index.html"),
            #html_template,
        ).expect("failed to write html");
        eprintln!(
            "exported to {}/ ({} files)",
            __gorbie_out.display(),
            3,
        );
    }
}

fn find_wasm_bindgen() -> Option<String> {
    if let Ok(path) = which("wasm-bindgen") {
        return Some(path);
    }

    let home = std::env::var("HOME").ok()?;

    let cargo_bin = format!("{home}/.cargo/bin/wasm-bindgen");
    if std::path::Path::new(&cargo_bin).exists() {
        return Some(cargo_bin);
    }

    let cache_dir = if cfg!(target_os = "macos") {
        format!("{home}/Library/Caches/dev.trunkrs.trunk")
    } else {
        format!("{home}/.cache/dev.trunkrs.trunk")
    };
    if let Ok(entries) = std::fs::read_dir(&cache_dir) {
        let mut best: Option<String> = None;
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if name.starts_with("wasm-bindgen-") {
                let candidate = format!("{}/{}/wasm-bindgen", cache_dir, name);
                if std::path::Path::new(&candidate).exists() {
                    best = Some(candidate);
                }
            }
        }
        if best.is_some() {
            return best;
        }
    }

    None
}

fn which(name: &str) -> std::result::Result<String, ()> {
    let output = std::process::Command::new("which")
        .arg(name)
        .output()
        .map_err(|_| ())?;
    if output.status.success() {
        let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if !path.is_empty() {
            return Ok(path);
        }
    }
    Err(())
}

fn gorbie_path() -> proc_macro2::TokenStream {
    // Resolve the package, not a particular dependency key. Cargo users may
    // keep `GORBIE = { package = "lissajous", ... }` or choose another alias.
    match crate_name("lissajous") {
        Ok(FoundCrate::Itself) => {
            if is_library_crate() {
                quote!(crate)
            } else {
                let ident = Ident::new(&package_name(), Span::call_site());
                quote!(::#ident)
            }
        }
        Ok(FoundCrate::Name(name)) => {
            let ident = Ident::new(&name.replace('-', "_"), Span::call_site());
            quote!(::#ident)
        }
        Err(_) => {
            let ident = Ident::new(&package_name(), Span::call_site());
            quote!(::#ident)
        }
    }
}

fn is_library_crate() -> bool {
    let crate_name = std::env::var("CARGO_CRATE_NAME").ok();
    let package_name = std::env::var("CARGO_PKG_NAME").ok();
    crate_name.is_some() && crate_name == package_name
}

fn package_name() -> String {
    std::env::var("CARGO_PKG_NAME")
        .unwrap_or_else(|_| "lissajous".to_owned())
        .replace('-', "_")
}
