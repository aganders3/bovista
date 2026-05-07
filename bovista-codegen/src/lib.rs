//! Procedural macros for Bovista bindings code generation
//!
//! This crate provides macros to reduce boilerplate when creating Python and WASM
//! bindings for Visual types.

use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use syn::{parse_macro_input, ItemImpl, ImplItem, ImplItemFn, FnArg, ReturnType, Type, parse_quote};

/// Generates method implementations for Visual wrappers
///
/// This macro takes method signatures and generates the boilerplate code to:
/// 1. Downcast the VisualRef to the specific Visual type
/// 2. Call the method on the downcasted visual
/// 3. Handle errors appropriately for the binding (PyO3 or wasm-bindgen)
///
/// # Example (Python)
///
/// ```ignore
/// #[visual_methods(ImageVisual)]
/// #[pymethods]
/// impl PyImageVisual {
///     fn set_slice_z(&self, z: f32) -> PyResult<()> {}
///     fn get_stats(&self) -> PyResult<(usize, usize)> {}
/// }
/// ```
#[proc_macro_attribute]
pub fn visual_methods(attr: TokenStream, item: TokenStream) -> TokenStream {
    let visual_type = parse_macro_input!(attr as syn::Type);
    let input = parse_macro_input!(item as ItemImpl);

    // Detect binding type from return types
    let binding_type = detect_binding_type(&input);

    let mut new_items = Vec::new();

    for item in input.items {
        match item {
            ImplItem::Fn(method) => {
                if method.block.stmts.is_empty() {
                    let expanded = expand_visual_method(&method, &visual_type, &binding_type);
                    new_items.push(ImplItem::Fn(expanded));
                } else {
                    new_items.push(ImplItem::Fn(method));
                }
            }
            other => new_items.push(other),
        }
    }

    let output = ItemImpl { items: new_items, ..input };
    TokenStream::from(quote! { #output })
}

/// Generates camera and scene delegation methods for Viewer wrappers.
///
/// Apply to a `#[pymethods] impl PyViewer` or `#[wasm_bindgen] impl JsViewer` block.
/// Methods with empty bodies are filled in with the appropriate `self.camera.*` or
/// `self.scene.*` delegation. Methods with non-empty bodies are kept as-is.
///
/// Supported empty-body method names:
/// - Camera: `set_camera_position`, `set_camera_target`, `set_camera_up`,
///   `orbit_camera`, `pan_camera`, `zoom_camera`, `set_camera_clip_planes`,
///   `set_camera_projection_mode`, `get_camera_projection_mode`,
///   `set_camera_ortho_height`, `get_camera_ortho_height`, `get_camera_distance`
/// - Scene: `visual_count`, `clear_visuals`, `clear_scene`
///
/// # Example
///
/// ```ignore
/// #[camera_methods]
/// #[pymethods]
/// impl PyViewer {
///     fn set_camera_position(&mut self, x: f32, y: f32, z: f32) {}
///     fn orbit_camera(&mut self, delta_x: f32, delta_y: f32) {}
///     fn visual_count(&self) -> usize {}
///     // ...
/// }
/// ```
#[proc_macro_attribute]
pub fn camera_methods(_attr: TokenStream, item: TokenStream) -> TokenStream {
    let input = parse_macro_input!(item as ItemImpl);

    let mut new_items = Vec::new();

    for item in input.items {
        match item {
            ImplItem::Fn(method) => {
                if method.block.stmts.is_empty() {
                    let expanded = expand_camera_method(&method);
                    new_items.push(ImplItem::Fn(expanded));
                } else {
                    new_items.push(ImplItem::Fn(method));
                }
            }
            other => new_items.push(other),
        }
    }

    let output = ItemImpl { items: new_items, ..input };
    TokenStream::from(quote! { #output })
}

// ─── helpers ─────────────────────────────────────────────────────────────────

#[derive(Debug)]
enum BindingType {
    Python,
    Wasm,
}

fn detect_binding_type(input: &ItemImpl) -> BindingType {
    for item in &input.items {
        if let ImplItem::Fn(method) = item {
            if let ReturnType::Type(_, ty) = &method.sig.output {
                let type_str = quote!(#ty).to_string();
                if type_str.contains("PyResult") {
                    return BindingType::Python;
                } else if type_str.contains("JsValue") {
                    return BindingType::Wasm;
                }
            }
        }
    }
    BindingType::Python
}

fn extract_arg_names(method: &ImplItemFn) -> Vec<&syn::Ident> {
    method.sig.inputs.iter()
        .skip(1) // skip &self / &mut self
        .filter_map(|arg| {
            if let FnArg::Typed(pat_type) = arg {
                if let syn::Pat::Ident(pat_ident) = &*pat_type.pat {
                    return Some(&pat_ident.ident);
                }
            }
            None
        })
        .collect()
}

fn expand_visual_method(method: &ImplItemFn, visual_type: &Type, binding_type: &BindingType) -> ImplItemFn {
    let method_name = &method.sig.ident;
    let attrs = &method.attrs;
    let vis = &method.vis;
    let sig = &method.sig;
    let arg_names = extract_arg_names(method);

    let is_mutable = matches!(&method.sig.output, ReturnType::Default) || {
        if let ReturnType::Type(_, ty) = &method.sig.output {
            let s = quote!(#ty).to_string();
            s.contains("( )") || s.contains("()")
        } else {
            false
        }
    };
    let helper_fn = if is_mutable { quote! { with_visual_mut } } else { quote! { with_visual_ref } };

    let body: TokenStream2 = match binding_type {
        BindingType::Python => quote! {
            bindings_common::#helper_fn::<#visual_type, _, _>(
                &self.inner,
                |visual| visual.#method_name(#(#arg_names),*)
            ).map_err(|e| PyErr::new::<pyo3::exceptions::PyTypeError, _>(e))
        },
        BindingType::Wasm => quote! {
            bindings_common::#helper_fn::<#visual_type, _, _>(
                &self.inner,
                |visual| visual.#method_name(#(#arg_names),*)
            ).map_err(|e| JsValue::from_str(&e))
        },
    };

    parse_quote! {
        #(#attrs)*
        #vis #sig { #body }
    }
}

fn expand_camera_method(method: &ImplItemFn) -> ImplItemFn {
    let method_name = &method.sig.ident;
    let attrs = &method.attrs;
    let vis = &method.vis;
    let sig = &method.sig;
    let name_str = method_name.to_string();
    let arg_names = extract_arg_names(method);

    let body: TokenStream2 = match name_str.as_str() {
        "set_camera_position" => quote! {
            self.camera.position = glam::Vec3::new(#(#arg_names),*);
        },
        "set_camera_target" => quote! {
            self.camera.target = glam::Vec3::new(#(#arg_names),*);
        },
        "set_camera_up" => quote! {
            self.camera.up = glam::Vec3::new(#(#arg_names),*);
        },
        "orbit_camera" => {
            let (dx, dy) = (&arg_names[0], &arg_names[1]);
            quote! { self.camera.orbit(#dx, #dy); }
        },
        "pan_camera" => {
            let (dx, dy) = (&arg_names[0], &arg_names[1]);
            quote! { self.camera.pan(#dx, #dy); }
        },
        "zoom_camera" => {
            let delta = &arg_names[0];
            quote! { self.camera.zoom(#delta); }
        },
        "set_camera_clip_planes" => {
            let (near, far) = (&arg_names[0], &arg_names[1]);
            quote! {
                self.camera.near = #near;
                self.camera.far = #far;
            }
        },
        "set_camera_projection_mode" => {
            let mode = &arg_names[0];
            quote! { self.camera.set_projection_mode(#mode.into()); }
        },
        "get_camera_projection_mode" => quote! {
            self.camera.projection_mode().into()
        },
        "set_camera_ortho_height" => {
            let h = &arg_names[0];
            quote! { self.camera.set_ortho_height(#h); }
        },
        "get_camera_ortho_height" => quote! {
            self.camera.ortho_height()
        },
        "get_camera_distance" => quote! {
            (self.camera.position - self.camera.target).length()
        },
        "visual_count" => quote! {
            self.scene.len()
        },
        "clear_visuals" | "clear_scene" => quote! {
            self.scene.clear();
        },
        other => {
            // Unknown method name — emit a compile error
            let msg = format!("camera_methods: no implementation known for `{}`", other);
            quote! { compile_error!(#msg); }
        }
    };

    parse_quote! {
        #(#attrs)*
        #vis #sig { #body }
    }
}
