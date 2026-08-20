; Definitions
(function_definition name: (identifier) @name) @def.function
(class_definition name: (identifier) @name) @def.class

; Imports
(import_statement name: (dotted_name) @path) @import
(import_statement name: (aliased_import name: (dotted_name) @path)) @import
(import_from_statement module_name: (dotted_name) @path) @import
(import_from_statement module_name: (relative_import) @path) @import

; Calls
(call function: (identifier) @callee) @call
(call function: (attribute attribute: (identifier) @callee)) @call
