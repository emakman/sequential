#[proc_macro]
pub fn actor(u: proc_macro::TokenStream) -> proc_macro::TokenStream {
    let Actor {
        before_struct,
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
    let mut new_args = vec![];
    let (msg_enum, msg_to_action, method_to_msg) =
        match process_actions(&actor, &msg_type, &mut impl_, &mut new_args) {
            Ok(ok) => ok,
            Err(e) => return e.to_compile_error().into(),
        };
    let new_arg_names = new_args.iter().filter_map(|a| {
        if let syn::FnArg::Typed(syn::PatType { pat: a, .. }) = a {
            let syn::Pat::Ident(a) = &**a else {
                unreachable!()
            };
            Some(a)
        } else {
            None
        }
    });
    let r = quote::quote! {
        #[derive(Clone)]
        #vis struct #handle_type #impl_generics(::sequential::tokio::sync::mpsc::UnboundedSender<#msg_type #type_generics>) #where_clause;
        const _: () = {
            #(#before_struct)*
            #struct_
            #(#before_impl)*
            #impl_ impl #impl_generics #handle_type #type_generics {
                fn new(#(#new_args)*) -> Self {
                    let (#tx, mut #rx) = ::sequential::tokio::sync::mpsc::unbounded_channel::<#msg_type #type_generics>();
                            ::sequential::tokio::spawn(async move {
                                let mut #actor = <#actor_type #type_generics>::new(#(#new_arg_names)*);
                                while let Some(msg) = #rx.recv().await {
                                    match msg {
                                        #msg_to_action
                                    }
                                }
                            });
                    Self(#tx)
                }
                #method_to_msg
            }
            #(#at_end)*
        };
        #[allow(non_camel_case_types)]
        enum #msg_type #impl_generics #where_clause {
            #msg_enum
        }
    };
    //println!("{r}");
    r.into()
}

struct Actor {
    before_struct: Vec<syn::Item>,
    struct_: syn::ItemStruct,
    before_impl: Vec<syn::Item>,
    impl_: syn::ItemImpl,
    at_end: Vec<syn::Item>,
}
impl syn::parse::Parse for Actor {
    fn parse(input: &syn::parse::ParseBuffer) -> Result<Self, syn::Error> {
        let mut before_struct = vec![];
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
        Ok(Self {
            before_struct,
            struct_: struct_.ok_or(syn::Error::new(
                input.span(),
                "No `struct` definition in #[actor] module.",
            ))?,
            before_impl,
            impl_: impl_.ok_or(syn::Error::new(
                input.span(),
                "No `impl` for #[actor] struct.",
            ))?,
            at_end,
        })
    }
}

fn process_actions(
    actor: &syn::Ident,
    msg_type: &syn::Ident,
    impl_: &mut syn::ItemImpl,
    new_args: &mut Vec<syn::FnArg>,
) -> Result<
    (
        proc_macro2::TokenStream,
        proc_macro2::TokenStream,
        proc_macro2::TokenStream,
    ),
    syn::Error,
> {
    let mut messages = vec![];
    for item in &mut impl_.items {
        if let syn::ImplItem::Fn(syn::ImplItemFn { attrs, sig, .. }) = item {
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
                        messages.push((returns, item.vis.clone(), item.sig.clone()));
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
                        messages.push((returns, item.vis.clone(), item.sig.clone()));
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
                        messages.push((returns, item.vis.clone(), item.sig.clone()));
                    }
                }
            } else if sig.ident == "new" {
                if let Some(r) = sig.receiver() {
                    return Err(syn::Error::new_spanned(
                        r,
                        "associated function `new` must be a constructor.",
                    ));
                }
                for input in &sig.inputs {
                    if let syn::FnArg::Typed(syn::PatType { pat, .. }) = input {
                        if !matches!(&**pat, syn::Pat::Ident(_)) {
                            return Err(syn::Error::new_spanned(
                                pat,
                                "Pattern arguments are not allowed on associated function `new`.",
                            ));
                        }
                    }
                }
                new_args.extend(sig.inputs.iter().cloned());
            }
        }
    }
    let msg_enum = messages
        .iter()
        .map(|(_, _, syn::Signature { ident, inputs, .. })| {
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
        .map(|(_, _, syn::Signature { ident, inputs, .. })| {
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
            quote::quote! {
                #vis #asyncness #fn_token #ident(#inputs) #output {
                    #channel
                    let _ = self.0.send(#msg_type::#ident(#(#input_names,)*#channel_arg));
                    #return_
                }
            }
        },
    );
    Ok((
        quote::quote! {#(#msg_enum),*},
        quote::quote! {#(#msg_to_action),*},
        quote::quote! {#(#method_to_msg)*},
    ))
}
