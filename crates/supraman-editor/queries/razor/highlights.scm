(identifier) @variable

(comment) @comment
(razor_comment) @comment
(html_comment) @comment

[
  "at_page" "at_using" "at_model" "at_inject" "at_layout" "at_inherits" "at_implements" "at_namespace"
  "at_attribute" "at_typeparam" "at_rendermode" "at_preservewhitespace" "at_section"
] @preproc
[
  "at_if" "at_for" "at_foreach" "at_while" "at_do" "at_switch" "at_try" "at_lock" "at_await"
  "at_block" "at_explicit" "at_implicit" "at_colon_transition" "at_at_escape"
] @keyword
(razor_rendermode) @constant
(razor_attribute_name) @attribute

[
  "abstract" "add" "alias" "and" "as" "ascending" "async" "await" "base" "break" "break;" "by" "case" "catch"
  "checked" "class" "const" "continue" "default" "delegate" "descending" "do" "else" "else if" "enum" "equals"
  "event" "explicit" "extern" "file" "finally" "fixed" "for" "foreach" "from" "get" "global" "goto" "group"
  "if" "implicit" "in" "init" "interface" "internal" "into" "is" "join" "let" "lock" "namespace" "new" "not"
  "notnull" "on" "operator" "or" "orderby" "out" "override" "params" "partial" "private" "protected" "public"
  "readonly" "record" "ref" "remove" "required" "return" "scoped" "sealed" "select" "set" "sizeof" "stackalloc"
  "static" "struct" "switch" "this" "throw" "try" "typeof" "unchecked" "unmanaged" "unsafe" "using" "var"
  "virtual" "volatile" "when" "where" "while" "with" "yield"
] @keyword
(modifier) @keyword
(implicit_type) @keyword

[
  "#define" "#elif" "#else" "#endif" "#endregion" "#error" "#if" "#line" "#nullable" "#pragma" "#region"
  "#undef" "#warning"
] @preproc

(string_literal) @string
(verbatim_string_literal) @string
(raw_string_literal) @string
(interpolated_string_expression) @string
(character_literal) @string
(escape_sequence) @string.escape

(integer_literal) @number
(real_literal) @number
(boolean_literal) @boolean
(null_literal) @constant

(predefined_type) @type.builtin
(class_declaration name: (identifier) @type)
(struct_declaration name: (identifier) @type)
(record_declaration name: (identifier) @type)
(interface_declaration name: (identifier) @type)
(enum_declaration name: (identifier) @type)
(type_parameter name: (identifier) @type)
(generic_name (identifier) @type)
(qualified_name (identifier) @type)
(type_argument_list (identifier) @type)
(base_list (identifier) @type)
(variable_declaration type: (identifier) @type)
(parameter type: (identifier) @type)
(property_declaration type: (identifier) @type)
(method_declaration returns: (identifier) @type)
(object_creation_expression type: (identifier) @type)
(razor_model_directive name: (identifier) @type)
(razor_inherits_directive name: (identifier) @type)
(razor_layout_directive name: (identifier) @type)
(enum_member_declaration name: (identifier) @constant)

(member_access_expression name: (identifier) @property)
(property_declaration name: (identifier) @property)
(method_declaration name: (identifier) @function.definition)
(local_function_statement name: (identifier) @function.definition)
(constructor_declaration name: (identifier) @constructor)
(invocation_expression function: (identifier) @function)
(invocation_expression function: (member_access_expression name: (identifier) @function.method))
(parameter name: (identifier) @variable.parameter)
(attribute name: (identifier) @attribute)

[
  "!" "!=" "%" "%=" "&" "&&" "&=" "*" "*=" "+" "++" "+=" "-" "--" "-=" "->" ".." "/" "/=" "<<" "<<=" "<="
  "=" "==" "=>" ">=" ">>" ">>=" ">>>" ">>>=" "?" "??" "??=" "^" "^=" "|" "|=" "||" "~"
] @operator

["<" ">" "</" "/>"] @punctuation.bracket
["(" ")" "[" "]" "{" "}"] @punctuation.bracket
["," ";" "." ":" "::"] @punctuation.delimiter
(interpolation_brace) @punctuation.special
