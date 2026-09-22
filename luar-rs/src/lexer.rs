#[derive(Debug, Clone, PartialEq)]
pub enum TokenKind {
    // Luar keywords
    Class,
    Is,
    Public,
    Private,
    Static,
    Abstract,
    Override,
    Final,
    Super,
    Operator,
    Import,
    Declare,
    Global,
    // Lua/Luau keywords
    Function,
    End,
    Local,
    Return,
    Self_,
    If,
    Then,
    Else,
    ElseIf,
    While,
    For,
    Do,
    Repeat,
    Until,
    In,
    Break,
    Continue,
    Goto,
    And,
    Or,
    Not,
    True,
    False,
    Nil,
    // Literals
    Number,
    LuaString,
    TemplateString,
    Ident,
    // Delimiters
    LParen,
    RParen,
    LBrace,
    RBrace,
    LBracket,
    RBracket,
    // Punctuation
    Dot,
    Comma,
    Colon,
    Bind,
    DoubleColon,
    Semicolon,
    Arrow,
    // Assignment & comparison
    Eq,
    EqEq,
    TildeEq,
    Lt,
    Gt,
    LtEq,
    GtEq,
    // Arithmetic
    Plus,
    Minus,
    Star,
    Slash,
    SlashSlash,
    Percent,
    Caret,
    Hash,
    // String concat & vararg
    DotDot,
    DotDotDot,
    // Optional type marker
    Question,
    Eof,
}

#[derive(Debug, Clone)]
pub struct Token {
    pub kind: TokenKind,
    pub value: String,
    pub line: usize,
    /// One-based UTF-16 positions, matching the LSP coordinate system.
    pub column: usize,
    pub end_line: usize,
    pub end_column: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SourceSpan {
    pub line: usize,
    pub column: usize,
    pub end_line: usize,
    pub end_column: usize,
}

impl Token {
    pub fn span(&self) -> SourceSpan {
        SourceSpan {
            line: self.line,
            column: self.column,
            end_line: self.end_line,
            end_column: self.end_column,
        }
    }
}

pub struct Lexer {
    src: Vec<char>,
    pos: usize,
    line: usize,
    column: usize,
}

impl Lexer {
    pub fn new(src: &str) -> Self {
        Lexer {
            src: src.chars().collect(),
            pos: 0,
            line: 1,
            column: 1,
        }
    }

    pub fn tokenize(&mut self) -> Result<Vec<Token>, String> {
        let mut tokens = Vec::new();
        loop {
            self.skip_whitespace_and_comments()?;
            if self.pos >= self.src.len() {
                tokens.push(Token {
                    kind: TokenKind::Eof,
                    value: String::new(),
                    line: self.line,
                    column: self.column,
                    end_line: self.line,
                    end_column: self.column,
                });
                break;
            }
            let tok = self.next_token()?;
            tokens.push(tok);
        }
        Ok(tokens)
    }

    fn peek(&self) -> char {
        self.src.get(self.pos).copied().unwrap_or('\0')
    }
    fn peek2(&self) -> char {
        self.src.get(self.pos + 1).copied().unwrap_or('\0')
    }
    fn advance(&mut self) -> char {
        let c = self.src[self.pos];
        self.pos += 1;
        if c == '\n' {
            self.line += 1;
            self.column = 1;
        } else {
            self.column += c.len_utf16();
        }
        c
    }

    fn skip_whitespace_and_comments(&mut self) -> Result<(), String> {
        loop {
            while self.pos < self.src.len() && self.peek().is_ascii_whitespace() {
                self.advance();
            }
            if self.peek() == '-' && self.peek2() == '-' {
                self.advance();
                self.advance();
                if self.peek() == '[' {
                    let level = self.count_long_bracket();
                    if level >= 0 {
                        self.read_long_string(level as usize)?;
                        continue;
                    }
                }
                while self.pos < self.src.len() && self.peek() != '\n' {
                    self.advance();
                }
            } else {
                break;
            }
        }
        Ok(())
    }

    fn count_long_bracket(&self) -> i32 {
        let mut i = self.pos;
        if self.src.get(i) != Some(&'[') {
            return -1;
        }
        i += 1;
        let mut level = 0i32;
        while self.src.get(i) == Some(&'=') {
            level += 1;
            i += 1;
        }
        if self.src.get(i) == Some(&'[') {
            level
        } else {
            -1
        }
    }

    fn read_long_string(&mut self, level: usize) -> Result<String, String> {
        // consume opening [=*[
        for _ in 0..(level + 2) {
            self.advance();
        }
        if self.peek() == '\n' {
            self.advance();
        }
        let mut result = String::new();
        loop {
            if self.pos >= self.src.len() {
                return Err(format!("unfinished long string at line {}", self.line));
            }
            let c = self.advance();
            if c == ']' {
                let mut eq = 0;
                while self.peek() == '=' {
                    eq += 1;
                    self.advance();
                }
                if eq == level && self.peek() == ']' {
                    self.advance();
                    return Ok(result);
                }
                result.push(']');
                for _ in 0..eq {
                    result.push('=');
                }
            } else {
                result.push(c);
            }
        }
    }

    fn next_token(&mut self) -> Result<Token, String> {
        let line = self.line;
        let column = self.column;
        let c = self.peek();

        // Long strings
        if c == '[' && (self.peek2() == '[' || self.peek2() == '=') {
            let level = self.count_long_bracket();
            if level >= 0 {
                let s = self.read_long_string(level as usize)?;
                return Ok(Token {
                    kind: TokenKind::LuaString,
                    value: s,
                    line,
                    column,
                    end_line: self.line,
                    end_column: self.column,
                });
            }
        }

        // Strings
        if c == '"' || c == '\'' {
            return self.read_string(c, line, column);
        }

        // Template strings (backtick)
        if c == '`' {
            return self.read_template_string(line, column);
        }

        // Numbers
        if c.is_ascii_digit() || (c == '.' && self.peek2().is_ascii_digit()) {
            return Ok(self.read_number(line, column));
        }

        // Identifiers and keywords
        if c.is_alphabetic() || c == '_' {
            return Ok(self.read_ident_or_keyword(line, column));
        }

        // Operators and punctuation
        self.advance();
        let kind = match c {
            '(' => TokenKind::LParen,
            ')' => TokenKind::RParen,
            '{' => TokenKind::LBrace,
            '}' => TokenKind::RBrace,
            '[' => TokenKind::LBracket,
            ']' => TokenKind::RBracket,
            ',' => TokenKind::Comma,
            ';' => TokenKind::Semicolon,
            '#' => TokenKind::Hash,
            '+' => TokenKind::Plus,
            '-' => {
                if self.peek() == '>' {
                    self.advance();
                    TokenKind::Arrow
                } else {
                    TokenKind::Minus
                }
            }
            '*' => TokenKind::Star,
            '%' => TokenKind::Percent,
            '^' => TokenKind::Caret,
            '?' => TokenKind::Question,
            '/' => {
                if self.peek() == '/' {
                    self.advance();
                    TokenKind::SlashSlash
                } else {
                    TokenKind::Slash
                }
            }
            '.' => {
                if self.peek() == '.' {
                    self.advance();
                    if self.peek() == '.' {
                        self.advance();
                        TokenKind::DotDotDot
                    } else {
                        TokenKind::DotDot
                    }
                } else {
                    TokenKind::Dot
                }
            }
            ':' => {
                if self.peek() == ':' {
                    self.advance();
                    TokenKind::DoubleColon
                } else if self.peek() == '=' {
                    self.advance();
                    TokenKind::Bind
                } else {
                    TokenKind::Colon
                }
            }
            '=' => {
                if self.peek() == '=' {
                    self.advance();
                    TokenKind::EqEq
                } else {
                    TokenKind::Eq
                }
            }
            '~' => {
                if self.peek() == '=' {
                    self.advance();
                    TokenKind::TildeEq
                } else {
                    return Err(format!("unexpected char '~' at line {line}"));
                }
            }
            '<' => {
                if self.peek() == '=' {
                    self.advance();
                    TokenKind::LtEq
                } else {
                    TokenKind::Lt
                }
            }
            '>' => {
                if self.peek() == '=' {
                    self.advance();
                    TokenKind::GtEq
                } else {
                    TokenKind::Gt
                }
            }
            _ => return Err(format!("unexpected character '{c}' at line {line}")),
        };
        Ok(Token {
            kind,
            value: c.to_string(),
            line,
            column,
            end_line: self.line,
            end_column: self.column,
        })
    }

    fn read_string(&mut self, quote: char, line: usize, column: usize) -> Result<Token, String> {
        self.advance(); // opening quote
        let mut s = String::new();
        loop {
            if self.pos >= self.src.len() {
                return Err(format!("unfinished string at line {line}"));
            }
            let c = self.advance();
            if c == quote {
                break;
            }
            if c == '\\' {
                let e = self.advance();
                match e {
                    'n' => s.push('\n'),
                    't' => s.push('\t'),
                    'r' => s.push('\r'),
                    '\\' => s.push('\\'),
                    '\'' => s.push('\''),
                    '"' => s.push('"'),
                    _ => {
                        s.push('\\');
                        s.push(e);
                    }
                }
            } else {
                s.push(c);
            }
        }
        Ok(Token {
            kind: TokenKind::LuaString,
            value: s,
            line,
            column,
            end_line: self.line,
            end_column: self.column,
        })
    }

    fn read_template_string(&mut self, line: usize, column: usize) -> Result<Token, String> {
        self.advance(); // opening `
        let mut s = String::new();
        let mut escaped = false;
        loop {
            if self.pos >= self.src.len() {
                return Err(format!("unfinished template string at line {line}"));
            }
            let c = self.advance();
            if escaped {
                s.push('\\');
                s.push(c);
                escaped = false;
                continue;
            }
            if c == '\\' {
                escaped = true;
                continue;
            }
            if c == '`' {
                break;
            }
            s.push(c);
        }
        Ok(Token {
            kind: TokenKind::TemplateString,
            value: s,
            line,
            column,
            end_line: self.line,
            end_column: self.column,
        })
    }

    fn read_number(&mut self, line: usize, column: usize) -> Token {
        let mut s = String::new();
        while self.pos < self.src.len()
            && (self.peek().is_ascii_alphanumeric() || self.peek() == '.' || self.peek() == '_')
        {
            s.push(self.advance());
        }
        Token {
            kind: TokenKind::Number,
            value: s,
            line,
            column,
            end_line: self.line,
            end_column: self.column,
        }
    }

    fn read_ident_or_keyword(&mut self, line: usize, column: usize) -> Token {
        let mut s = String::new();
        while self.pos < self.src.len() && (self.peek().is_alphanumeric() || self.peek() == '_') {
            s.push(self.advance());
        }
        let kind = match s.as_str() {
            "class" => TokenKind::Class,
            "is" => TokenKind::Is,
            "public" => TokenKind::Public,
            "private" => TokenKind::Private,
            "static" => TokenKind::Static,
            "abstract" => TokenKind::Abstract,
            "override" => TokenKind::Override,
            "final" => TokenKind::Final,
            "super" => TokenKind::Super,
            "operator" => TokenKind::Operator,
            "import" => TokenKind::Import,
            "declare" => TokenKind::Declare,
            "global" => TokenKind::Global,
            "function" => TokenKind::Function,
            "end" => TokenKind::End,
            "local" => TokenKind::Local,
            "return" => TokenKind::Return,
            "self" => TokenKind::Self_,
            "if" => TokenKind::If,
            "then" => TokenKind::Then,
            "else" => TokenKind::Else,
            "elseif" => TokenKind::ElseIf,
            "while" => TokenKind::While,
            "for" => TokenKind::For,
            "do" => TokenKind::Do,
            "repeat" => TokenKind::Repeat,
            "until" => TokenKind::Until,
            "in" => TokenKind::In,
            "break" => TokenKind::Break,
            "continue" => TokenKind::Continue,
            "goto" => TokenKind::Goto,
            "and" => TokenKind::And,
            "or" => TokenKind::Or,
            "not" => TokenKind::Not,
            "true" => TokenKind::True,
            "false" => TokenKind::False,
            "nil" => TokenKind::Nil,
            _ => TokenKind::Ident,
        };
        Token {
            kind,
            value: s,
            line,
            column,
            end_line: self.line,
            end_column: self.column,
        }
    }
}
