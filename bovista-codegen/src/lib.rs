//! Procedural macros for Bovista bindings code generation
//!
//! This crate provides macros to reduce boilerplate when creating Python and WASM
//! bindings for Visual types.

use proc_macro::TokenStream;
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
///     fn set_slice_z(&self, z: f32) -> PyResult<()>;
///     fn get_stats(&self) -> PyResult<(usize, usize)>;
/// }
/// ```
///
/// Expands to:
///
/// ```ignore
/// #[pymethods]
/// impl PyImageVisual {
///     fn set_slice_z(&self, z: f32) -> PyResult<()> {
///         bindings_common::with_visual_mut::<ImageVisual, _, _>(
///             &self.inner,
///             |visual| visual.set_slice_z(z)
///         ).map_err(|e| PyErr::new::<pyo3::exceptions::PyTypeError, _>(e))
///     }
///
///     fn get_stats(&self) -> PyResult<(usize, usize)> {
///         bindings_common::with_visual_ref::<ImageVisual, _, _>(
///             &self.inner,
///             |visual| visual.get_stats()
///         ).map_err(|e| PyErr::new::<pyo3::exceptions::PyTypeError, _>(e))
///     }
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
                // Only process methods with empty bodies (signature-only)
                if method.block.stmts.is_empty() {
                    let expanded = expand_method(&method, &visual_type, &binding_type);
                    new_items.push(ImplItem::Fn(expanded));
                } else {
                    // Keep methods with implementations as-is (factory methods, etc.)
                    new_items.push(ImplItem::Fn(method));
                }
            }
            other => new_items.push(other),
        }
    }

    let output = ItemImpl {
        items: new_items,
        ..input
    };

    TokenStream::from(quote! { #output })
}

#[derive(Debug)]
enum BindingType {
    Python,
    Wasm,
}

fn detect_binding_type(input: &ItemImpl) -> BindingType {
    // Look at return types to determine if this is Python or WASM
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

    // Default to Python
    BindingType::Python
}

fn expand_method(method: &ImplItemFn, visual_type: &Type, binding_type: &BindingType) -> ImplItemFn {
    let method_name = &method.sig.ident;
    let attrs = &method.attrs;
    let vis = &method.vis;
    let sig = &method.sig;

    // Extract argument names (skip &self)
    let arg_names: Vec<_> = method.sig.inputs.iter()
        .skip(1) // Skip &self
        .filter_map(|arg| {
            if let FnArg::Typed(pat_type) = arg {
                if let syn::Pat::Ident(pat_ident) = &*pat_type.pat {
                    return Some(&pat_ident.ident);
                }
            }
            None
        })
        .collect();

    // Determine if method is mutable (returns ()) or immutable (returns value)
    let is_mutable = matches!(&method.sig.output, ReturnType::Default);
    let helper_fn = if is_mutable {
        quote! { with_visual_mut }
    } else {
        // Check if return type is PyResult<()> or Result<(), JsValue> (also mutable)
        if let ReturnType::Type(_, ty) = &method.sig.output {
            let type_str = quote!(#ty).to_string();
            if type_str.contains("( )") || type_str.contains("()") {
                quote! { with_visual_mut }
            } else {
                quote! { with_visual_ref }
            }
        } else {
            quote! { with_visual_mut }
        }
    };

    // Generate the method body
    let body = match binding_type {
        BindingType::Python => {
            quote! {
                bindings_common::#helper_fn::<#visual_type, _, _>(
                    &self.inner,
                    |visual| visual.#method_name(#(#arg_names),*)
                ).map_err(|e| PyErr::new::<pyo3::exceptions::PyTypeError, _>(e))
            }
        }
        BindingType::Wasm => {
            quote! {
                bindings_common::#helper_fn::<#visual_type, _, _>(
                    &self.inner,
                    |visual| visual.#method_name(#(#arg_names),*)
                ).map_err(|e| JsValue::from_str(&e))
            }
        }
    };

    parse_quote! {
        #(#attrs)*
        #vis #sig {
            #body
        }
    }
}
