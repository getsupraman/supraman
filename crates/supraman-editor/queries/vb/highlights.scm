; The grammar's keywords are hidden tokens no pattern can name: the file is a keyword, every node below it takes its own colour.
(source_file) @keyword

(identifier) @variable

(comment) @comment
(preprocessor_directive) @preproc

(string_literal) @string
(interpolated_string_literal) @string
(character_literal) @string
(date_literal) @string.special
["\"" "$\""] @string

(integer_literal) @number
(floating_point_literal) @number
(boolean_literal) @boolean

(modifier) @keyword
(primitive_type) @type.builtin
(namespace_name (identifier) @type)

(class_block name: (identifier) @type)
(module_block name: (identifier) @type)
(structure_block name: (identifier) @type)
(interface_block name: (identifier) @type)
(enum_block name: (identifier) @type)
(delegate_declaration name: (identifier) @type)
(type_parameter name: (identifier) @type)
(enum_member name: (identifier) @constant)
(const_declaration name: (identifier) @constant)

(member_access member: (identifier) @property)
(property_declaration name: (identifier) @property)
(event_declaration name: (identifier) @property)
(member_initializer member: (identifier) @property)

(method_declaration name: (identifier) @function.definition)
(invocation target: (identifier) @function)
(invocation target: (member_access member: (identifier) @function.method))

(parameter name: (identifier) @variable.parameter)
(lambda_parameter name: (identifier) @variable.parameter)
(argument name: (identifier) @variable.parameter)

(attribute name: (identifier) @attribute)
(attribute name: (namespace_name (identifier) @attribute))

(label_statement label: (identifier) @label)
(goto_statement label: (identifier) @label)

(relational_operator) @operator
[
  "=" "<>" "<" ">" "<=" ">=" "+" "-" "*" "/" "\\" "^" "<<" ">>"
  "+=" "-=" "*=" "/=" "\\=" "^=" "&=" "<<=" ">>="
] @operator

["(" ")" "{" "}"] @punctuation.bracket
["," "." ":"] @punctuation.delimiter
(interpolation ["{" "}"] @punctuation.special)
