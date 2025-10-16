extern crate proc_macro;

use proc_macro::TokenStream;
use quote::{format_ident, quote};
use syn::{Ident, ItemStruct, parse_macro_input};

#[proc_macro_attribute]
pub fn lift_struct(_attr: TokenStream, item: TokenStream) -> TokenStream {
    let input = parse_macro_input!(item as ItemStruct);
    let name = &input.ident;
    let vis = &input.vis;
    let generics = &input.generics;

    let fields = if let syn::Fields::Named(fields_named) = &input.fields {
        &fields_named.named
    } else {
        panic!("lift_struct can only be applied to structs with named fields");
    };

    let mut signal_fields = Vec::new();
    let mut non_signal_fields = Vec::new();

    for field in fields.iter() {
        let has_signal_attr = field
            .attrs
            .iter()
            .any(|attr| attr.path().is_ident("signal"));
        if has_signal_attr {
            signal_fields.push(field);
        } else {
            non_signal_fields.push(field);
        }
    }

    let lifted_name = Ident::new(&format!("{}_", name), name.span());

    let signal_field_names: Vec<_> = signal_fields.iter().map(|f| &f.ident).collect();
    let signal_field_types: Vec<_> = signal_fields.iter().map(|f| &f.ty).collect();
    let non_signal_field_names: Vec<_> = non_signal_fields.iter().map(|f| &f.ident).collect();
    let non_signal_field_types: Vec<_> = non_signal_fields.iter().map(|f| &f.ty).collect();

    let signal_ref_names: Vec<_> = signal_field_names
        .iter()
        .map(|name| format_ident!("ref_{}", name.as_ref().unwrap()))
        .collect();

    let signal_ver_names: Vec<_> = signal_field_names
        .iter()
        .map(|name| format_ident!("{}_ver", name.as_ref().unwrap()))
        .collect();

    let signal_changed_names: Vec<_> = signal_field_names
        .iter()
        .map(|name| format_ident!("{}_changed", name.as_ref().unwrap()))
        .collect();

    let signal_last_ver_names: Vec<_> = signal_field_names
        .iter()
        .map(|name| format_ident!("last_{}_ver", name.as_ref().unwrap()))
        .collect();

    let (_impl_generics, _ty_generics, where_clause) = generics.split_for_impl();

    let expanded = quote! {
        #vis struct #name<'a> {
            #(pub #signal_field_names: crate::signal::Signal<'a, #signal_field_types>,)*
            #(pub #non_signal_field_names: #non_signal_field_types,)*
        }

        #[derive(Clone)]
        #vis struct #lifted_name #generics #where_clause {
            #(pub #signal_field_names: #signal_field_types,)*
            #(pub #non_signal_field_names: #non_signal_field_types,)*
        }

        impl<'a> crate::api::LiftInto<crate::signal::Signal<'a, #lifted_name>> for #name<'a>
        where
            #(#signal_field_types: Clone + 'a,)*
        {
            fn lift(self) -> crate::signal::Signal<'a, #lifted_name> {
                use std::cell::RefCell;
                use std::rc::Rc;
                use crate::api::RcRef;

                let initial = #lifted_name {
                    #(#signal_field_names: self.#signal_field_names.0.value.borrow().clone(),)*
                    #(#non_signal_field_names: self.#non_signal_field_names,)*
                };

                let new_struct = crate::signal::Signal::new(initial);

                #(
                    {
                        let ref_result = RcRef::new(Rc::clone(&new_struct.0), false);
                        let #signal_ref_names = RcRef::new(Rc::clone(&self.#signal_field_names.0), false);
                        let #signal_last_ver_names = RefCell::new(0u64);

                        self.#signal_field_names.0.subscribers.borrow_mut().push(Box::new(move || {
                            ref_result.upgrade()
                                .zip(#signal_ref_names.upgrade())
                                .map(|(result_inner, signal_inner)| {
                                    let #signal_ver_names = *signal_inner.version.borrow();
                                    let #signal_changed_names = #signal_ver_names != *#signal_last_ver_names.borrow();

                                    if #signal_changed_names {
                                        *#signal_last_ver_names.borrow_mut() = #signal_ver_names;

                                        let result = crate::signal::Signal(result_inner);
                                        result.__send_deferred_with(|s| {
                                            s.#signal_field_names = signal_inner.value.borrow().clone();
                                        });
                                    }
                                    true
                                })
                                .unwrap_or(false)
                        }) as Box<dyn Fn() -> bool + 'a>);
                    }
                )*

                new_struct
            }
        }

        impl<'a> crate::api::WeakLiftInto<crate::signal::Signal<'a, #lifted_name>> for #name<'a>
        where
            #(#signal_field_types: Clone + 'a,)*
        {
            fn weak_lift(self) -> crate::signal::Signal<'a, #lifted_name> {
                use std::cell::RefCell;
                use std::rc::Rc;
                use crate::api::RcRef;

                let initial = #lifted_name {
                    #(#signal_field_names: self.#signal_field_names.0.value.borrow().clone(),)*
                    #(#non_signal_field_names: self.#non_signal_field_names,)*
                };

                let new_struct = crate::signal::Signal::new(initial);

                #(
                    {
                        let ref_result = RcRef::new(Rc::clone(&new_struct.0), true);
                        let #signal_ref_names = RcRef::new(Rc::clone(&self.#signal_field_names.0), true);
                        let #signal_last_ver_names = RefCell::new(0u64);

                        self.#signal_field_names.0.subscribers.borrow_mut().push(Box::new(move || {
                            ref_result.upgrade()
                                .zip(#signal_ref_names.upgrade())
                                .map(|(result_inner, signal_inner)| {
                                    let #signal_ver_names = *signal_inner.version.borrow();
                                    let #signal_changed_names = #signal_ver_names != *#signal_last_ver_names.borrow();

                                    if #signal_changed_names {
                                        *#signal_last_ver_names.borrow_mut() = #signal_ver_names;

                                        let result = crate::signal::Signal(result_inner);
                                        result.__send_deferred_with(|s| {
                                            s.#signal_field_names = signal_inner.value.borrow().clone();
                                        });
                                    }
                                    true
                                })
                                .unwrap_or(false)
                        }) as Box<dyn Fn() -> bool + 'a>);
                    }
                )*

                new_struct
            }
        }
    };

    TokenStream::from(expanded)
}

#[proc_macro_derive(Lift, attributes(signal))]
pub fn derive_lift(_input: TokenStream) -> TokenStream {
    TokenStream::from(quote! {
        compile_error!("Use #[lift_struct] attribute macro instead of #[derive(Lift)]");
    })
}
