// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License in the LICENSE-APACHE file or at:
//     https://www.apache.org/licenses/LICENSE-2.0

#![recursion_limit = "128"]
#![feature(proc_macro_diagnostic)]

extern crate proc_macro;

mod args;

use std::collections::HashMap;

use proc_macro2::{Span, TokenStream};
use quote::{quote, ToTokens, TokenStreamExt};
use std::fmt::Write;
use syn::punctuated::Punctuated;
use syn::spanned::Spanned;
use syn::token::Comma;
use syn::Token;
use syn::{parse_macro_input, parse_quote};
use syn::{
    DeriveInput, FnArg, GenericParam, Generics, Ident, ImplItemMethod, Type, TypeParam, TypePath,
};

use self::args::ChildType;

mod layout;

struct SubstTyGenerics<'a>(&'a Generics, HashMap<Ident, Type>);

// impl copied from syn, with modifications
impl<'a> ToTokens for SubstTyGenerics<'a> {
    fn to_tokens(&self, tokens: &mut TokenStream) {
        if self.0.params.is_empty() {
            return;
        }

        <Token![<]>::default().to_tokens(tokens);

        // Print lifetimes before types and consts, regardless of their
        // order in self.params.
        //
        // TODO: ordering rules for const parameters vs type parameters have
        // not been settled yet. https://github.com/rust-lang/rust/issues/44580
        let mut trailing_or_empty = true;
        for param in self.0.params.pairs() {
            if let GenericParam::Lifetime(def) = *param.value() {
                // Leave off the lifetime bounds and attributes
                def.lifetime.to_tokens(tokens);
                param.punct().to_tokens(tokens);
                trailing_or_empty = param.punct().is_some();
            }
        }
        for param in self.0.params.pairs() {
            if let GenericParam::Lifetime(_) = **param.value() {
                continue;
            }
            if !trailing_or_empty {
                <Token![,]>::default().to_tokens(tokens);
                trailing_or_empty = true;
            }
            match *param.value() {
                GenericParam::Lifetime(_) => unreachable!(),
                GenericParam::Type(param) => {
                    if let Some(result) = self.1.get(&param.ident) {
                        result.to_tokens(tokens);
                    } else {
                        param.ident.to_tokens(tokens);
                    }
                }
                GenericParam::Const(param) => {
                    // Leave off the const parameter defaults
                    param.ident.to_tokens(tokens);
                }
            }
            param.punct().to_tokens(tokens);
        }

        <Token![>]>::default().to_tokens(tokens);
    }
}

/// Macro to derive widget traits
///
/// See the [`kas::macros`](../kas/macros/index.html) module documentation.
#[proc_macro_derive(Widget, attributes(core, widget, layout, handler, layout_data))]
pub fn derive(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    let mut ast = parse_macro_input!(input as DeriveInput);

    let mut args = match args::read_attrs(&mut ast) {
        Ok(w) => w,
        Err(err) => return err.to_compile_error().into(),
    };
    let (impl_generics, ty_generics, where_clause) = ast.generics.split_for_impl();
    let name = &ast.ident;
    let widget_name = name.to_string();

    let core = args.core;
    let count = args.children.len();

    let mut get_rules = quote! {};
    let mut get_mut_rules = quote! {};
    let mut walk_rules = quote! {};
    let mut walk_mut_rules = quote! {};
    for (i, child) in args.children.iter().enumerate() {
        let ident = &child.ident;
        get_rules.append_all(quote! { #i => Some(&self.#ident), });
        get_mut_rules.append_all(quote! { #i => Some(&mut self.#ident), });
        walk_rules.append_all(quote! { self.#ident.walk(f); });
        walk_mut_rules.append_all(quote! { self.#ident.walk_mut(f); });
    }

    let mut toks = quote! {
        impl #impl_generics kas::WidgetCore
            for #name #ty_generics #where_clause
        {
            fn core_data(&self) -> &kas::CoreData {
                &self.#core
            }

            fn core_data_mut(&mut self) -> &mut kas::CoreData {
                &mut self.#core
            }

            fn widget_name(&self) -> &'static str {
                #widget_name
            }

            fn as_widget(&self) -> &dyn kas::Widget { self }
            fn as_widget_mut(&mut self) -> &mut dyn kas::Widget { self }

            fn len(&self) -> usize {
                #count
            }
            fn get(&self, _index: usize) -> Option<&dyn kas::Widget> {
                match _index {
                    #get_rules
                    _ => None
                }
            }
            fn get_mut(&mut self, _index: usize) -> Option<&mut dyn kas::Widget> {
                match _index {
                    #get_mut_rules
                    _ => None
                }
            }
            fn walk(&self, f: &mut dyn FnMut(&dyn kas::Widget)) {
                #walk_rules
                f(self);
            }
            fn walk_mut(&mut self, f: &mut dyn FnMut(&mut dyn kas::Widget)) {
                #walk_mut_rules
                f(self);
            }
        }
    };

    if let Some(layout) = args.layout {
        let (fns, dt) = match layout::derive(&args.children, layout, &args.layout_data) {
            Ok(res) => res,
            Err(err) => return err.to_compile_error().into(),
        };
        toks.append_all(quote! {
            impl #impl_generics kas::Layout
                    for #name #ty_generics #where_clause
            {
                #fns
            }
            impl #impl_generics kas::LayoutData
                    for #name #ty_generics #where_clause
            {
                #dt
            }
        });
    }

    if let Some(_) = args.widget {
        toks.append_all(quote! {
            impl #impl_generics kas::Widget
                    for #name #ty_generics #where_clause
            {
            }
        });
    }

    for handler in args.handler.drain(..) {
        let msg = handler.msg;
        let subs = handler.substitutions;
        let mut generics = ast.generics.clone();
        generics.params = generics
            .params
            .into_pairs()
            .filter(|pair| match pair.value() {
                &GenericParam::Type(TypeParam { ref ident, .. }) => !subs.contains_key(ident),
                _ => true,
            })
            .collect();
        /* Problem: bounded_ty is too generic with no way to extract the Ident
        if let Some(clause) = &mut generics.where_clause {
            clause.predicates = clause.predicates
                .into_pairs()
                .filter(|pair| match pair.value() {
                    &WherePredicate::Type(PredicateType { ref bounded_ty, .. }) =>
                        subs.iter().all(|pair| &pair.0 != ident),
                    _ => true,
                })
                .collect();
        }
        */
        if !handler.generics.params.is_empty() {
            if !generics.params.empty_or_trailing() {
                generics.params.push_punct(Default::default());
            }
            generics.params.extend(handler.generics.params.into_pairs());
        }
        if let Some(h_clauses) = handler.generics.where_clause {
            if let Some(ref mut clauses) = generics.where_clause {
                if !clauses.predicates.empty_or_trailing() {
                    clauses.predicates.push_punct(Default::default());
                }
                clauses.predicates.extend(h_clauses.predicates.into_pairs());
            } else {
                generics.where_clause = Some(h_clauses);
            }
        }
        // Note: we may have extra generic types used in where clauses, but we
        // don't want these in ty_generics.
        let (impl_generics, _, where_clause) = generics.split_for_impl();
        let ty_generics = SubstTyGenerics(&ast.generics, subs);

        let mut ev_to_num = TokenStream::new();
        for child in args.children.iter() {
            let ident = &child.ident;
            let handler = if let Some(ref h) = child.args.handler {
                quote! { r.try_into().unwrap_or_else(|msg| self.#h(mgr, msg)) }
            } else {
                quote! { r.into() }
            };
            ev_to_num.append_all(quote! {
                if id <= self.#ident.id() {
                    let r = self.#ident.handle(mgr, id, event);
                    #handler
                } else
            });
        }

        let handler = if args.children.is_empty() {
            // rely on the default implementation
            quote! {}
        } else {
            quote! {
                fn handle(&mut self, mgr: &mut kas::event::Manager, id: kas::WidgetId, event: kas::event::Event)
                -> kas::event::Response<Self::Msg>
                {
                    use kas::{WidgetCore, event::Response};
                    #ev_to_num {
                        debug_assert!(id == self.id(), "Handler::handle: bad WidgetId");
                        Response::Unhandled(event)
                    }
                }
            }
        };

        toks.append_all(quote! {
            impl #impl_generics kas::event::Handler
                    for #name #ty_generics #where_clause
            {
                type Msg = #msg;
                #handler
            }
        });
    }

    toks.into()
}

/// Macro to create a widget with anonymous type
///
/// See the [`kas::macros`](../kas/macros/index.html) module documentation.
///
/// Currently usage of this macro requires `#![feature(proc_macro_hygiene)]`.
#[proc_macro]
pub fn make_widget(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    let mut find_handler_ty_buf: Vec<(Ident, Type)> = vec![];
    // find type of handler's message; return None on error
    let mut find_handler_ty = |handler: &Ident,
                               impls: &Vec<(Option<TypePath>, Vec<ImplItemMethod>)>|
     -> Option<Type> {
        // check the buffer in case we did this already
        for (ident, ty) in &find_handler_ty_buf {
            if ident == handler {
                return Some(ty.clone());
            }
        }

        let mut x: Option<(Ident, Type)> = None;

        for impl_block in impls {
            for f in &impl_block.1 {
                if f.sig.ident == *handler {
                    if let Some(x) = x {
                        handler
                            .span()
                            .unwrap()
                            .error("multiple methods with this name")
                            .emit();
                        x.0.span()
                            .unwrap()
                            .error("first method with this name")
                            .emit();
                        f.sig
                            .ident
                            .span()
                            .unwrap()
                            .error("second method with this name")
                            .emit();
                        return None;
                    }
                    if f.sig.inputs.len() != 3 {
                        f.sig.span()
                            .unwrap()
                            .error("handler functions must have signature: fn handler(&mut self, mgr: &mut Manager, msg: T)")
                            .emit();
                        return None;
                    }
                    let arg = f.sig.inputs.last().unwrap();
                    let ty = match arg {
                        FnArg::Typed(arg) => (*arg.ty).clone(),
                        _ => panic!("expected typed argument"), // nothing else is possible here?
                    };
                    x = Some((f.sig.ident.clone(), ty));
                }
            }
        }
        if let Some(x) = x {
            find_handler_ty_buf.push((handler.clone(), x.1.clone()));
            Some(x.1)
        } else {
            handler
                .span()
                .unwrap()
                .error("no methods with this name found")
                .emit();
            None
        }
    };

    let mut args = parse_macro_input!(input as args::MakeWidget);

    // Used to make fresh identifiers for generic types
    let mut name_buf = String::with_capacity(32);

    // fields of anonymous struct:
    let mut field_toks = quote! {
        #[core] core: kas::CoreData,
        #[layout_data] layout_data: <Self as kas::LayoutData>::Data,
    };
    // initialisers for these fields:
    let mut field_val_toks = quote! {
        core: Default::default(),
        layout_data: Default::default(),
    };
    // debug impl
    let mut debug_fields = TokenStream::new();

    // extra generic types and where clause for handler impl
    let mut handler_extra = Punctuated::<_, Comma>::new();
    let mut handler_clauses = Punctuated::<_, Comma>::new();

    let msg = &args.handler_msg;
    let extra_attrs = args.extra_attrs;

    for (index, field) in args.fields.drain(..).enumerate() {
        let attr = field.widget_attr;

        let ident = match &field.ident {
            Some(ref ident) => ident.clone(),
            None => {
                name_buf.clear();
                name_buf
                    .write_fmt(format_args!("mw_anon_{}", index))
                    .unwrap();
                Ident::new(&name_buf, Span::call_site())
            }
        };

        let ty: Type = match field.ty {
            ChildType::Fixed(ty) => ty.clone(),
            ChildType::Generic(gen_msg, gen_bound) => {
                name_buf.clear();
                name_buf.write_fmt(format_args!("MWAnon{}", index)).unwrap();
                let ty = Ident::new(&name_buf, Span::call_site());

                if let Some(ref wattr) = attr {
                    if let Some(tyr) = gen_msg {
                        handler_clauses.push(quote! { #ty: kas::event::Handler<Msg = #tyr> });
                    } else {
                        // No typing. If a handler is specified, then the child must implement
                        // Handler<Msg = X> where the handler takes type X; otherwise
                        // we use `msg.into()` and this conversion must be supported.
                        if let Some(ref handler) = wattr.args.handler {
                            if let Some(ty_bound) = find_handler_ty(handler, &args.impls) {
                                handler_clauses
                                    .push(quote! { #ty: kas::event::Handler<Msg = #ty_bound> });
                            } else {
                                return quote! {}.into(); // exit after emitting error
                            }
                        } else {
                            name_buf.push_str("R");
                            let tyr = Ident::new(&name_buf, Span::call_site());
                            handler_extra.push(tyr.clone());
                            handler_clauses.push(quote! { #ty: kas::event::Handler<Msg = #tyr> });
                            handler_clauses.push(quote! { #msg: From<#tyr> });
                        }
                    }

                    if let Some(mut bound) = gen_bound {
                        bound.bounds.push(parse_quote! { kas::Widget });
                        args.generics.params.push(parse_quote! { #ty: #bound });
                    } else {
                        args.generics.params.push(parse_quote! { #ty: kas::Widget });
                    }
                } else {
                    args.generics.params.push(parse_quote! { #ty });
                }

                Type::Path(TypePath {
                    qself: None,
                    path: ty.into(),
                })
            }
        };

        let value = &field.value;

        field_toks.append_all(quote! { #attr #ident: #ty, });
        field_val_toks.append_all(quote! { #ident: #value, });
        debug_fields
            .append_all(quote! { write!(f, ", {}: {:?}", stringify!(#ident), self.#ident)?; });
    }

    let (impl_generics, ty_generics, where_clause) = args.generics.split_for_impl();

    let mut impls = quote! {};

    for impl_block in args.impls {
        let mut contents = TokenStream::new();
        for method in impl_block.1 {
            contents.append_all(std::iter::once(method));
        }
        let target = if let Some(t) = impl_block.0 {
            quote! { #t for }
        } else {
            quote! {}
        };
        impls.append_all(quote! {
            impl #impl_generics #target AnonWidget #ty_generics #where_clause {
                #contents
            }
        });
    }

    let handler_where = if handler_clauses.is_empty() {
        quote! {}
    } else {
        quote! { where #handler_clauses }
    };

    // TODO: we should probably not rely on recursive macro expansion here!
    // (I.e. use direct code generation for Widget derivation, instead of derive.)
    let toks = (quote! { {
        #[handler(msg = #msg, generics = < #handler_extra > #handler_where)]
        #extra_attrs
        #[derive(Clone, Debug, kas::macros::Widget)]
        struct AnonWidget #impl_generics #where_clause {
            #field_toks
        }

        #impls

        AnonWidget {
            #field_val_toks
        }
    } })
    .into();

    toks
}

/// Macro to derive `From<VoidMsg>`
///
/// See the [`kas::macros`](../kas/macros/index.html) module documentation.
#[proc_macro_derive(VoidMsg)]
pub fn derive_empty_msg(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    let ast = parse_macro_input!(input as DeriveInput);
    let (impl_generics, ty_generics, where_clause) = ast.generics.split_for_impl();
    let name = &ast.ident;

    let toks = quote! {
        impl #impl_generics From<kas::event::VoidMsg>
            for #name #ty_generics #where_clause
        {
            fn from(_: kas::event::VoidMsg) -> Self {
                unreachable!()
            }
        }
    };
    toks.into()
}
