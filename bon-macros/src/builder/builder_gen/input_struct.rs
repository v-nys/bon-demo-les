use crate::builder::builder_gen::{
    BuilderGenCtx, Field, FieldExpr, FinishFunc, FinishFuncBody, Generics, StartFunc,
};
use crate::builder::params::{BuilderParams, ItemParams};
use darling::FromMeta;
use itertools::Itertools;
use prox::prelude::*;
use quote::quote;
use syn::visit_mut::VisitMut;

#[derive(Debug, FromMeta)]
pub(crate) struct StructInputParams {
    #[darling(flatten)]
    base: BuilderParams,
    start_fn: Option<ItemParams>,
}

pub(crate) struct StructInputCtx {
    orig_struct: syn::ItemStruct,
    norm_struct: syn::ItemStruct,
    params: StructInputParams,
    struct_ty: syn::Type,
}

impl StructInputCtx {
    pub(crate) fn new(params: StructInputParams, orig_struct: syn::ItemStruct) -> Self {
        let generic_args = orig_struct
            .generics
            .params
            .iter()
            .map(super::generic_param_to_arg);
        let struct_ident = &orig_struct.ident;
        let struct_ty = syn::parse_quote!(#struct_ident<#(#generic_args),*>);

        let mut norm_struct = orig_struct.clone();

        // Structs are free to use `Self` inside of their trait bounds and any
        // internal type contexts.
        crate::normalization::NormalizeSelfTy {
            self_ty: &struct_ty,
        }
        .visit_item_struct_mut(&mut norm_struct);

        Self {
            orig_struct,
            norm_struct,
            params,
            struct_ty,
        }
    }

    fn builder_ident(&self) -> syn::Ident {
        if let Some(builder_type) = &self.params.base.builder_type {
            return builder_type.clone();
        }

        quote::format_ident!("{}Builder", self.norm_struct.ident)
    }

    pub(crate) fn adapted_struct(&self) -> syn::ItemStruct {
        let mut orig = self.orig_struct.clone();

        // Remove all `#[builder]` attributes from the struct since
        // we used them just to configure this macro, and are they
        // no longer needed in the output code
        orig.attrs.retain(|attr| !attr.path().is_ident("builder"));

        orig
    }

    pub(crate) fn into_builder_gen_ctx(self) -> Result<BuilderGenCtx> {
        let builder_ident = self.builder_ident();
        let builder_private_impl_ident = quote::format_ident!("__{builder_ident}PrivateImpl");
        let builder_state_trait_ident = quote::format_ident!("__{builder_ident}State");

        let fields = match self.norm_struct.fields {
            syn::Fields::Named(fields) => fields,
            _ => {
                prox::bail!(
                    &self.norm_struct,
                    "Only structs with named fields are supported"
                )
            }
        };

        let fields: Vec<_> = fields
            .named
            .iter()
            .map(Field::from_syn_field)
            .try_collect()?;

        let generics = Generics {
            params: Vec::from_iter(self.norm_struct.generics.params.iter().cloned()),
            where_clause: self.norm_struct.generics.where_clause.clone(),
        };

        let finish_func_body = StructLiteralBody {
            struct_ident: self.norm_struct.ident.clone(),
        };

        let ItemParams {
            name: start_func_ident,
            vis: start_func_vis,
        } = self.params.start_fn.unwrap_or_default();

        let start_func_ident = start_func_ident
            .unwrap_or_else(|| syn::Ident::new("builder", self.norm_struct.ident.span()));

        let finish_func_ident = self
            .params
            .base
            .finish_fn
            .unwrap_or_else(|| syn::Ident::new("build", start_func_ident.span()));

        let struct_ty = &self.struct_ty;
        let finish_func = FinishFunc {
            ident: finish_func_ident,
            unsafety: None,
            asyncness: None,
            body: Box::new(finish_func_body),
            output: syn::parse_quote!(-> #struct_ty),
        };

        let start_func_docs = format!(
            "Use builder syntax to create an instance of [`{}`]",
            self.norm_struct.ident
        );

        let start_func = StartFunc {
            ident: start_func_ident,
            vis: start_func_vis,
            attrs: vec![syn::parse_quote!(#[doc = #start_func_docs])],
            generics: None,
        };

        let ctx = BuilderGenCtx {
            fields,
            builder_ident,
            builder_private_impl_ident,
            builder_state_trait_ident,

            receiver: None,
            generics,
            vis: self.norm_struct.vis,

            start_func,
            finish_func,
        };

        Ok(ctx)
    }
}

struct StructLiteralBody {
    struct_ident: syn::Ident,
}

impl FinishFuncBody for StructLiteralBody {
    fn gen(&self, field_exprs: &[FieldExpr<'_>]) -> TokenStream2 {
        let Self { struct_ident } = self;

        let field_exprs = field_exprs.iter().map(|FieldExpr { field, expr }| {
            let ident = &field.ident;
            quote! {
                #ident: #expr
            }
        });

        quote! {
            #struct_ident {
                #(#field_exprs,)*
            }
        }
    }
}

impl Field {
    pub(crate) fn from_syn_field(field: &syn::Field) -> Result<Self> {
        let ident = field.ident.clone().ok_or_else(|| {
            prox::err!(
                &field,
                "Only structs with named fields are supported. \
                Please name all fields of the struct"
            )
        })?;

        Field::new(&field.attrs, ident, Box::new(field.ty.clone()))
    }
}