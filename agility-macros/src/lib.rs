extern crate proc_macro;

use proc_macro::TokenStream;
use quote::{format_ident, quote};
use syn::{DeriveInput, Fields, GenericArgument, PathArguments, Type, TypePath, parse_macro_input};

/// Helper function to check if a type is Signal<'a, T> and extract the inner type T
fn extract_signal_inner_type(ty: &Type) -> Option<&Type> {
    if let Type::Path(TypePath { path, .. }) = ty {
        // Get the last segment of the path (e.g., "Signal" from "crate::signal::Signal")
        let last_segment = path.segments.last()?;

        // Check if it's named "Signal"
        if last_segment.ident != "Signal" {
            return None;
        }

        // Extract the generic arguments
        if let PathArguments::AngleBracketed(args) = &last_segment.arguments {
            // Signal<'a, T> has two generic arguments: lifetime 'a and type T
            // We want to extract T (the second argument)
            let mut iter = args.args.iter();
            iter.next()?; // Skip the lifetime

            if let Some(GenericArgument::Type(inner_ty)) = iter.next() {
                return Some(inner_ty);
            }
        }
    }
    None
}

/// Helper function to check if a type is SignalSync<'a, T> and extract the inner type T
fn extract_signal_sync_inner_type(ty: &Type) -> Option<&Type> {
    if let Type::Path(TypePath { path, .. }) = ty {
        // Get the last segment of the path (e.g., "SignalSync" from "crate::signal_sync::SignalSync")
        let last_segment = path.segments.last()?;

        // Check if it's named "SignalSync"
        if last_segment.ident != "SignalSync" {
            return None;
        }

        // Extract the generic arguments
        if let PathArguments::AngleBracketed(args) = &last_segment.arguments {
            // SignalSync<'a, T> has two generic arguments: lifetime 'a and type T
            // We want to extract T (the second argument)
            let mut iter = args.args.iter();
            iter.next()?; // Skip the lifetime

            if let Some(GenericArgument::Type(inner_ty)) = iter.next() {
                return Some(inner_ty);
            }
        }
    }
    None
}

#[proc_macro_derive(Lift)]
pub fn derive_lift(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    let name = &input.ident;
    let generics = &input.generics;
    let vis = &input.vis;

    // Get the fields
    let fields = match &input.data {
        syn::Data::Struct(data) => match &data.fields {
            Fields::Named(fields) => &fields.named,
            _ => panic!("Lift only supports structs with named fields"),
        },
        _ => panic!("Lift can only be derived for structs"),
    };

    // Separate signal fields from regular fields by checking the type
    let mut signal_fields = Vec::new();
    let mut regular_fields = Vec::new();

    for field in fields {
        if extract_signal_inner_type(&field.ty).is_some() {
            signal_fields.push(field);
        } else {
            regular_fields.push(field);
        }
    }

    // Generate the inner struct name (prefixed with underscore)
    let inner_name = format_ident!("_{}", name);

    // Generate fields for the inner struct (unwrapped types)
    let inner_struct_fields = fields.iter().map(|field| {
        let field_name = &field.ident;
        let field_vis = &field.vis;

        // If it's a Signal<'a, T>, use T; otherwise use the original type
        let field_ty = if let Some(inner_ty) = extract_signal_inner_type(&field.ty) {
            inner_ty
        } else {
            &field.ty
        };

        quote! {
            #field_vis #field_name: #field_ty
        }
    });

    // Generate the reactive setup code for signal fields
    let reactive_setup = signal_fields.iter().map(|field| {
        let field_name = &field.ident;

        quote! {
            {
                let result_signal_weak = std::rc::Rc::downgrade(&result_signal.0);
                let source_for_closure = std::rc::Rc::downgrade(&instance.#field_name.0);
                let react_fn = Box::new(move || {
                    if let Some(result_sig) = result_signal_weak.upgrade() {
                        if !*result_sig.explicitly_modified.borrow() {
                            if let Some(source) = source_for_closure.upgrade() {
                                result_sig.value.borrow_mut().#field_name = source.value.borrow().clone();
                            }
                        }
                    }
                });
                let cloned_signal = instance.#field_name.clone();
                cloned_signal.0.react_fns.borrow_mut().push(react_fn);
                cloned_signal.0.successors.borrow_mut().push(crate::signal::WeakSignalRef::new(&result_signal));
            }
        }
    });

    // Generate the inner struct initialization from main struct
    let inner_from_main = signal_fields.iter().map(|field| {
        let field_name = &field.ident;
        quote! {
            #field_name: instance.#field_name.0.value.borrow().clone()
        }
    });

    let regular_from_main = regular_fields.iter().map(|field| {
        let field_name = &field.ident;
        quote! {
            #field_name: instance.#field_name.clone()
        }
    });

    // Generate Clone trait bounds for signal fields (using the unwrapped inner type)
    let signal_clone_bounds = signal_fields.iter().filter_map(|field| {
        extract_signal_inner_type(&field.ty).map(|inner_ty| {
            quote! { #inner_ty: Clone }
        })
    });

    // Generate Clone trait bounds for regular fields
    let regular_clone_bounds = regular_fields.iter().map(|field| {
        let field_ty = &field.ty;
        quote! { #field_ty: Clone }
    });

    // Extract generics for impl block
    let (impl_generics, ty_generics, where_clause) = generics.split_for_impl();

    // Create a version of generics without lifetimes for the inner struct
    let type_params = generics.type_params().map(|tp| &tp.ident);
    let inner_ty_generics = if generics.type_params().count() > 0 {
        quote! { <#(#type_params),*> }
    } else {
        quote! {}
    };

    let expanded = quote! {
        // Inner struct (unwrapped types)
        #[derive(Clone)]
        #vis struct #inner_name #inner_ty_generics {
            #(#inner_struct_fields),*
        }

        impl #impl_generics #name #ty_generics #where_clause {
            pub fn lift(self) -> crate::signal::Signal<'a, #inner_name #inner_ty_generics>
            where
                #(#signal_clone_bounds,)*
                #(#regular_clone_bounds,)*
            {
                let instance = self;
                let initial_inner = #inner_name {
                    #(#inner_from_main,)*
                    #(#regular_from_main),*
                };

                let result_signal = crate::signal::Signal::new(initial_inner);

                #(#reactive_setup)*

                result_signal
            }
        }
    };

    TokenStream::from(expanded)
}

#[proc_macro_derive(LiftSync)]
pub fn derive_lift_sync(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    let name = &input.ident;
    let generics = &input.generics;
    let vis = &input.vis;

    // Get the fields
    let fields = match &input.data {
        syn::Data::Struct(data) => match &data.fields {
            Fields::Named(fields) => &fields.named,
            _ => panic!("LiftSync only supports structs with named fields"),
        },
        _ => panic!("LiftSync can only be derived for structs"),
    };

    // Separate signal fields from regular fields by checking the type
    let mut signal_fields = Vec::new();
    let mut regular_fields = Vec::new();

    for field in fields {
        if extract_signal_sync_inner_type(&field.ty).is_some() {
            signal_fields.push(field);
        } else {
            regular_fields.push(field);
        }
    }

    // Generate the inner struct name (prefixed with underscore)
    let inner_name = format_ident!("_{}", name);

    // Generate fields for the inner struct (unwrapped types)
    let inner_struct_fields = fields.iter().map(|field| {
        let field_name = &field.ident;
        let field_vis = &field.vis;

        // If it's a SignalSync<'a, T>, use T; otherwise use the original type
        let field_ty = if let Some(inner_ty) = extract_signal_sync_inner_type(&field.ty) {
            inner_ty
        } else {
            &field.ty
        };

        quote! {
            #field_vis #field_name: #field_ty
        }
    });

    // Generate the reactive setup code for signal fields (thread-safe version)
    let reactive_setup = signal_fields.iter().map(|field| {
        let field_name = &field.ident;

        quote! {
            {
                let result_signal_weak = std::sync::Arc::downgrade(&result_signal.0);
                let source_for_closure = std::sync::Arc::downgrade(&instance.#field_name.0);
                let react_fn = Box::new(move || {
                    if let Some(result_sig) = result_signal_weak.upgrade() {
                        if !result_sig.explicitly_modified.load(std::sync::atomic::Ordering::Acquire) {
                            if let Some(source) = source_for_closure.upgrade() {
                                result_sig.value.lock().unwrap().#field_name = source.value.lock().unwrap().clone();
                            }
                        }
                    }
                });
                let cloned_signal = instance.#field_name.clone();
                cloned_signal.0.react_fns.write().unwrap().push(react_fn);
                cloned_signal.0.successors.write().unwrap().push(crate::signal_sync::WeakSignalRefSync::new(&result_signal));
            }
        }
    });

    // Generate the inner struct initialization from main struct
    let inner_from_main = signal_fields.iter().map(|field| {
        let field_name = &field.ident;
        quote! {
            #field_name: instance.#field_name.0.value.lock().unwrap().clone()
        }
    });

    let regular_from_main = regular_fields.iter().map(|field| {
        let field_name = &field.ident;
        quote! {
            #field_name: instance.#field_name.clone()
        }
    });

    // Generate Clone + Send + Sync trait bounds for signal fields (using the unwrapped inner type)
    let signal_clone_bounds = signal_fields.iter().filter_map(|field| {
        extract_signal_sync_inner_type(&field.ty).map(|inner_ty| {
            quote! { #inner_ty: Clone + Send + Sync }
        })
    });

    // Generate Clone trait bounds for regular fields
    let regular_clone_bounds = regular_fields.iter().map(|field| {
        let field_ty = &field.ty;
        quote! { #field_ty: Clone }
    });

    // Extract generics for impl block
    let (impl_generics, ty_generics, where_clause) = generics.split_for_impl();

    // Create a version of generics without lifetimes for the inner struct
    let type_params = generics.type_params().map(|tp| &tp.ident);
    let inner_ty_generics = if generics.type_params().count() > 0 {
        quote! { <#(#type_params),*> }
    } else {
        quote! {}
    };

    let expanded = quote! {
        // Inner struct (unwrapped types)
        #[derive(Clone)]
        #vis struct #inner_name #inner_ty_generics {
            #(#inner_struct_fields),*
        }

        impl #impl_generics #name #ty_generics #where_clause {
            pub fn lift(self) -> crate::signal_sync::SignalSync<'a, #inner_name #inner_ty_generics>
            where
                #(#signal_clone_bounds,)*
                #(#regular_clone_bounds,)*
            {
                let instance = self;
                let initial_inner = #inner_name {
                    #(#inner_from_main,)*
                    #(#regular_from_main),*
                };

                let result_signal = crate::signal_sync::SignalSync::new(initial_inner);

                #(#reactive_setup)*

                result_signal
            }
        }
    };

    TokenStream::from(expanded)
}
