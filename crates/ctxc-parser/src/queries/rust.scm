; Definitions
(function_item name: (identifier) @name) @def.function
(struct_item name: (type_identifier) @name) @def.struct
(enum_item name: (type_identifier) @name) @def.enum
(trait_item name: (type_identifier) @name) @def.trait
(type_item name: (type_identifier) @name) @def.type
(const_item name: (identifier) @name) @def.constant
(static_item name: (identifier) @name) @def.constant
(mod_item name: (identifier) @name) @def.module
(macro_definition name: (identifier) @name) @def.function
(impl_item type: (type_identifier) @name) @def.class

; Imports
(use_declaration argument: (_) @path) @import

; Calls
(call_expression function: (identifier) @callee) @call
(call_expression function: (scoped_identifier name: (identifier) @callee)) @call
(call_expression function: (field_expression field: (field_identifier) @callee)) @call
