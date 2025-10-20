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
                let result_signal_clone = result_signal.clone();
                let source_for_closure = std::rc::Rc::clone(&instance.#field_name.0);
                let react_fn = Box::new(move || {
                    if !*result_signal_clone.0.explicitly_modified.borrow() {
                        result_signal_clone.modify(|v| {
                            v.#field_name = source_for_closure.value.borrow().clone();
                        });
                    }
                });
                let cloned_signal = instance.#field_name.clone();
                cloned_signal.0.react_fns.borrow_mut().push(react_fn);
                cloned_signal.0.successors.borrow_mut().push(Box::new(result_signal.clone()));
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
