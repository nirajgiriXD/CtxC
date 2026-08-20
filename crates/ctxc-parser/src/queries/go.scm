; Definitions
(function_declaration name: (identifier) @name) @def.function
(method_declaration name: (field_identifier) @name) @def.method
(type_declaration (type_spec name: (type_identifier) @name type: (struct_type))) @def.struct
(type_declaration (type_spec name: (type_identifier) @name type: (interface_type))) @def.interface
(type_declaration (type_spec name: (type_identifier) @name)) @def.type
(const_spec name: (identifier) @name) @def.constant

; Imports
(import_spec path: (interpreted_string_literal) @path) @import

; Calls
(call_expression function: (identifier) @callee) @call
(call_expression function: (selector_expression field: (field_identifier) @callee)) @call
