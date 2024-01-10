#[proc_macro]
pub fn actor(u: proc_macro::TokenStream) -> proc_macro::TokenStream {
    let Actor {
        before_struct,
        docs,
        struct_,
        before_impl,
        mut impl_,
        at_end,
    } = syn::parse_macro_input!(u);
    let vis = &struct_.vis;
    let actor_type = &struct_.ident;
    let handle_type = proc_macro2::Ident::new(
        &format!("{}Handle", actor_type),
        proc_macro2::Span::call_site(),
    );
    let (impl_generics, type_generics, where_clause) = struct_.generics.split_for_impl();
    let rx = proc_macro2::Ident::new("rx", proc_macro2::Span::mixed_site());
    let tx = proc_macro2::Ident::new("tx", proc_macro2::Span::mixed_site());
    let actor = proc_macro2::Ident::new("actor", proc_macro2::Span::mixed_site());
    let msg_type = proc_macro2::Ident::new(
        &format!("{}Message", actor_type),
        proc_macro2::Span::mixed_site(),
    );
    let (msg_enum, msg_to_action, method_to_msg, new_sig, new_arg_names, new_brace_span) =
        match process_actions(&actor, &msg_type, &mut impl_) {
            Ok(ok) => ok,
            Err(e) => return e.to_compile_error().into(),
        };
    let new = NewConstructor {
        tx: &tx,
        rx: &rx,
        actor: &actor,
        msg_type: &msg_type,
        actor_type: &actor_type,
        type_generics: &type_generics,
        sig: &new_sig,
        new_arg_names: &new_arg_names,
        msg_to_action: &msg_to_action,
        brace: syn::token::Brace(new_brace_span),
    };
    let r = quote::quote! {
        #[derive(Clone)]
        #(#docs)*
        #vis struct #handle_type #impl_generics(::sequential::tokio::sync::mpsc::UnboundedSender<#msg_type #type_generics>) #where_clause;
        const _: () = {
            #(#before_struct)*
            #struct_
            #(#before_impl)*
            #impl_ impl #impl_generics #handle_type #type_generics {
                #new
                #method_to_msg
            }
            #(#at_end)*
        };
        #[allow(non_camel_case_types)]
        enum #msg_type #impl_generics #where_clause {
            #msg_enum
        }
    };
    r.into()
}

struct NewConstructor<'a> {
    tx: &'a syn::Ident,
    rx: &'a syn::Ident,
    actor: &'a syn::Ident,
    msg_type: &'a syn::Ident,
    actor_type: &'a syn::Ident,
    type_generics: &'a syn::TypeGenerics<'a>,
    new_arg_names: &'a [syn::Ident],
    msg_to_action: &'a proc_macro2::TokenStream,
    sig: &'a proc_macro2::TokenStream,
    brace: syn::token::Brace,
}
impl<'a> quote::ToTokens for NewConstructor<'a> {
    fn to_tokens(&self, toks: &mut proc_macro2::TokenStream) {
        self.sig.to_tokens(toks);
        self.brace.surround(toks, |toks| {
            let Self { tx, rx, msg_type, actor, actor_type, type_generics, new_arg_names, msg_to_action, ..} = self;
            let body = quote::quote! {
                let (#tx, mut #rx) = ::sequential::tokio::sync::mpsc::unbounded_channel::<#msg_type #type_generics>();
                ::sequential::tokio::spawn(async move {
                    let mut #actor = <#actor_type #type_generics>::new(#(#new_arg_names),*);
                    while let Some(msg) = #rx.recv().await {
                        match msg {
                            #msg_to_action
                        }
                    }
                });
                Self(#tx)
            };
            body.to_tokens(toks);
        })
    }
}

struct Actor {
    before_struct: Vec<syn::Item>,
    docs: Vec<syn::Attribute>,
    struct_: syn::ItemStruct,
    before_impl: Vec<syn::Item>,
    impl_: syn::ItemImpl,
    at_end: Vec<syn::Item>,
}
impl syn::parse::Parse for Actor {
    fn parse(input: &syn::parse::ParseBuffer) -> Result<Self, syn::Error> {
        let mut before_struct = vec![];
        let mut docs = vec![];
        let mut struct_ = None;
        let mut before_impl = vec![];
        let mut impl_ = None;
        let mut at_end = vec![];
        while !input.is_empty() {
            match input.parse()? {
                syn::Item::Struct(s) if struct_.is_none() => struct_ = Some(s),
                syn::Item::Impl(i)
                    if impl_.is_none()
                        && i.trait_.is_none()
                        && struct_.as_ref().is_some_and(
                            |s| matches!(&*i.self_ty, syn::Type::Path(syn::TypePath {path,..}) if path.segments.len() == 1 && path.segments[0].ident == s.ident),
                        ) => impl_ = Some(i),
                item if struct_.is_none() => before_struct.push(item),
                item if impl_.is_none() => before_impl.push(item),
                item => at_end.push(item),
            }
        }
        let mut struct_ = struct_.ok_or(syn::Error::new(
            input.span(),
            "No `struct` definition in #[actor] module.",
        ))?;
        let mut i = 0;
        while i < struct_.attrs.len() {
            if struct_.attrs[i].path().is_ident("doc") {
                docs.push(struct_.attrs.remove(i));
            } else {
                i += 1;
            }
        }
        Ok(Self {
            before_struct,
            docs,
            struct_,
            before_impl,
            impl_: impl_.ok_or(syn::Error::new(
                input.span(),
                "No `impl` for #[actor] struct.",
            ))?,
            at_end,
        })
    }
}

fn process_actions<'a>(
    actor: &syn::Ident,
    msg_type: &syn::Ident,
    impl_: &'a mut syn::ItemImpl,
) -> Result<
    (
        proc_macro2::TokenStream,
        proc_macro2::TokenStream,
        proc_macro2::TokenStream,
        proc_macro2::TokenStream,
        Vec<syn::Ident>,
        proc_macro2::extra::DelimSpan,
    ),
    syn::Error,
> {
    let mut messages = vec![];
    let mut new_sig = proc_macro2::TokenStream::new();
    let mut new_arg_names = vec![];
    let mut new_span = None;
    for item in impl_.items.iter_mut() {
        if let syn::ImplItem::Fn(syn::ImplItemFn {
            attrs, sig, block, ..
        }) = item
        {
            let brace_span = block.brace_token.span;
            if let Some((idx, _)) = attrs
                .iter()
                .enumerate()
                .find(|(_, attr)| attr.path().is_ident("action"))
            {
                let attr = attrs.remove(idx);
                if sig.asyncness.is_none() {
                    return Err(syn::Error::new_spanned(
                        sig.fn_token,
                        "Actions must by async",
                    ));
                }
                if sig.receiver().is_none() {
                    return Err(syn::Error::new_spanned(sig, "Actions must be methods."));
                }
                for input in &sig.inputs {
                    if let syn::FnArg::Typed(syn::PatType { pat, .. }) = input {
                        if !matches!(&**pat, syn::Pat::Ident(_)) {
                            return Err(syn::Error::new_spanned(
                                pat,
                                "Pattern arguments are not allowed in actions.",
                            ));
                        }
                    }
                }

                enum MethodType {
                    Generator,
                    Multi,
                    Single,
                }
                let mut method_type = MethodType::Single;
                if !matches!(attr.meta, syn::Meta::Path(_)) {
                    attr.parse_nested_meta(|meta| {
                        if meta.path.is_ident("generator") {
                            method_type = MethodType::Generator;
                            Ok(())
                        } else if meta.path.is_ident("multi") {
                            method_type = MethodType::Multi;
                            Ok(())
                        } else {
                            Err(syn::Error::new_spanned(
                                meta.path,
                                "Unrecognized action property.",
                            ))
                        }
                    })?;
                };
                let method_type = method_type;
                let syn::ImplItem::Fn(item) = item else {
                    unreachable!()
                };
                match method_type {
                    MethodType::Multi => {
                        let returns = if let syn::ReturnType::Type(_, t) = &item.sig.output {
                            let t = t.clone();
                            struct YieldFolder;
                            impl syn::fold::Fold for YieldFolder {
                                fn fold_expr(&mut self, e: syn::Expr) -> syn::Expr {
                                    let out = proc_macro2::Ident::new(
                                        "output",
                                        proc_macro2::Span::mixed_site(),
                                    );
                                    if let syn::Expr::Yield(syn::ExprYield { expr, .. }) = e {
                                        syn::parse_quote! {
                                            {
                                                let _ = #out.send(#expr);
                                                if #out.is_closed() { return }
                                            }
                                        }
                                    } else {
                                        syn::fold::fold_expr(self, e)
                                    }
                                }
                            }

                            let out =
                                proc_macro2::Ident::new("output", proc_macro2::Span::mixed_site());
                            item.sig.inputs.push(syn::FnArg::Typed(syn::parse_quote! {
                                #out: ::sequential::tokio::sync::mpsc::UnboundedSender<#t>
                            }));
                            item.sig.output = syn::ReturnType::Default;
                            item.block =
                                syn::fold::fold_block(&mut YieldFolder, item.block.clone());
                            item.block.stmts.insert(
                                0,
                                syn::parse_quote! {
                                    {
                                        if #out.is_closed() { return }
                                    }
                                },
                            );
                            Some((
                                syn::parse_quote! {&self},
                                quote::quote! {-> impl ::sequential::futures::Stream<Item = #t>},
                                quote::quote! {
                                    let (tx, rx) = ::sequential::tokio::sync::mpsc::unbounded_channel();
                                },
                                quote::quote! {
                                    ::sequential::tokio_stream::wrappers::UnboundedReceiverStream::new(rx)
                                },
                            ))
                        } else {
                            None
                        };
                        messages.push((returns, item.vis.clone(), item.sig.clone(), brace_span));
                    }
                    MethodType::Generator => {
                        let returns = if let syn::ReturnType::Type(_, t) = &item.sig.output {
                            let t = t.clone();
                            struct YieldFolder;
                            impl syn::fold::Fold for YieldFolder {
                                fn fold_expr(&mut self, e: syn::Expr) -> syn::Expr {
                                    let out = proc_macro2::Ident::new(
                                        "output",
                                        proc_macro2::Span::mixed_site(),
                                    );
                                    if let syn::Expr::Yield(syn::ExprYield { expr, .. }) = e {
                                        syn::parse_quote! {
                                            {
                                                let _ = #out.send(Some(#expr)).await;
                                                let _ = #out.send(None).await;
                                                if #out.is_closed() { return }
                                            }
                                        }
                                    } else {
                                        syn::fold::fold_expr(self, e)
                                    }
                                }
                            }

                            let out =
                                proc_macro2::Ident::new("output", proc_macro2::Span::mixed_site());
                            item.sig.inputs.push(syn::FnArg::Typed(syn::parse_quote! {
                                #out: ::sequential::tokio::sync::mpsc::Sender<Option<#t>>
                            }));
                            item.sig.output = syn::ReturnType::Default;
                            item.block =
                                syn::fold::fold_block(&mut YieldFolder, item.block.clone());
                            item.block.stmts.insert(
                                0,
                                syn::parse_quote! {
                                    {
                                        let _ = #out.send(None).await;
                                        if #out.is_closed() { return }
                                    }
                                },
                            );
                            Some((
                                syn::parse_quote! {self},
                                quote::quote! {-> ::sequential::ResponseStream<Self,#t>},
                                quote::quote! {
                                    let (tx, rx) = ::sequential::tokio::sync::mpsc::channel(1);
                                },
                                quote::quote! {
                                    ::sequential::ResponseStream::new(self, rx)
                                },
                            ))
                        } else {
                            None
                        };
                        messages.push((returns, item.vis.clone(), item.sig.clone(), brace_span));
                    }
                    MethodType::Single => {
                        let returns = if let syn::ReturnType::Type(_, t) = &item.sig.output {
                            let t = t.clone();
                            let out =
                                proc_macro2::Ident::new("output", proc_macro2::Span::mixed_site());
                            item.sig.inputs.push(syn::FnArg::Typed(syn::parse_quote! {
                                #out: ::sequential::tokio::sync::oneshot::Sender<#t>
                            }));
                            item.sig.output = syn::ReturnType::Default;
                            let mut block = syn::Block {
                                brace_token: Default::default(),
                                stmts: vec![],
                            };
                            std::mem::swap(&mut block, &mut item.block);
                            item.block
                                .stmts
                                .push(syn::parse_quote!(let _ = #out.send((|| {#block})());));
                            Some((
                                syn::parse_quote! {&self},
                                quote::quote! {-> #t},
                                quote::quote! {
                                    let (tx, rx) = ::sequential::tokio::sync::oneshot::channel();
                                },
                                quote::quote! {
                                    rx.await.unwrap()
                                },
                            ))
                        } else {
                            None
                        };
                        messages.push((returns, item.vis.clone(), item.sig.clone(), brace_span));
                    }
                }
            } else if sig.ident == "new" {
                if let Some(r) = sig.receiver() {
                    return Err(syn::Error::new_spanned(
                        r,
                        "associated function `new` must be a constructor.",
                    ));
                }

                use quote::ToTokens;
                for input in &sig.inputs {
                    if let syn::FnArg::Typed(syn::PatType { pat, .. }) = input {
                        if let syn::Pat::Ident(p) = &**pat {
                            new_arg_names.push(p.ident.clone());
                        } else {
                            return Err(syn::Error::new_spanned(
                                pat,
                                "Pattern arguments are not allowed on associated function `new`.",
                            ));
                        }
                    }
                }
                for attr in attrs {
                    attr.to_tokens(&mut new_sig);
                }
                sig.fn_token.to_tokens(&mut new_sig);
                sig.ident.to_tokens(&mut new_sig);
                sig.paren_token.surround(&mut new_sig, |new_sig| {
                    for input in sig.inputs.pairs() {
                        input.value().to_tokens(new_sig);
                        input.punct().to_tokens(new_sig)
                    }
                });
                if let syn::ReturnType::Type(r, _) = &sig.output {
                    r.to_tokens(&mut new_sig);
                } else {
                    <syn::Token![->]>::default().to_tokens(&mut new_sig);
                }
                <syn::Token![Self]>::default().to_tokens(&mut new_sig);
                new_span = Some(block.brace_token.span);
            }
        }
    }
    let msg_enum = messages
        .iter()
        .map(|(_, _, syn::Signature { ident, inputs, .. }, _)| {
            let types = inputs.iter().filter_map(|i| {
                if let syn::FnArg::Typed(syn::PatType { ty, .. }) = i {
                    Some(ty)
                } else {
                    None
                }
            });
            quote::quote! {#ident(#(#types),*)}
        });
    let msg_to_action = messages
        .iter()
        .map(|(_, _, syn::Signature { ident, inputs, .. }, _)| {
            let input_names = inputs.iter().filter_map(|a| {
                if let syn::FnArg::Typed(syn::PatType { pat: a, .. }) = a {
                    let syn::Pat::Ident(a) = &**a else {
                        unreachable!()
                    };
                    Some(a)
                } else {
                    None
                }
            });
            let input_names2 = input_names.clone();
            quote::quote! {
                #msg_type::#ident(#(#input_names),*) => {#actor.#ident(#(#input_names2),*).await}
            }
        });
    let method_to_msg = messages.iter().map(
        |(
            out,
            vis,
            syn::Signature {
                ident,
                inputs,
                asyncness,
                fn_token,
                ..
            },
            brace_span,
        )| {
            let mut inputs = inputs.clone();
            let (receiver, output, channel, return_) = if let Some((r, o, c, r_)) = &out {
                (Some(r), Some(o), Some(c), Some(r_))
            } else {
                (None, None, None, None)
            };
            if let Some(syn::FnArg::Receiver(r)) = inputs.iter_mut().next() {
                *r = receiver.cloned().unwrap_or(syn::parse_quote!(&self));
            }
            if out.is_some() {
                inputs.pop();
            }
            let input_names = inputs.iter().filter_map(|a| {
                if let syn::FnArg::Typed(syn::PatType { pat: a, .. }) = a {
                    let syn::Pat::Ident(a) = &**a else {
                        unreachable!()
                    };
                    Some(a)
                } else {
                    None
                }
            });
            let channel_arg = out.as_ref().map(|_| {
                quote::quote! {tx}
            });
            let brace = syn::token::Brace(*brace_span);
            let mut r = quote::quote! {
            #vis #asyncness #fn_token #ident(#inputs) #output};
            brace.surround(&mut r, |r| {
                use quote::ToTokens;
                quote::quote! {
                    #channel
                    let _ = self.0.send(#msg_type::#ident(#(#input_names,)*#channel_arg));
                    #return_
                }
                .to_tokens(r)
            });
            r
        },
    );
    Ok((
        quote::quote! {#(#msg_enum),*},
        quote::quote! {#(#msg_to_action),*},
        quote::quote! {#(#method_to_msg)*},
        new_sig,
        new_arg_names,
        new_span.ok_or(syn::Error::new_spanned(
            &impl_,
            "Missing `new()` constructor.",
        ))?,
    ))
}
