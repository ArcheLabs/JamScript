import ts from "typescript5/lib/typescript.js";

const FIXED = new Set(["u8", "u16", "u32", "u64", "u128"]);
const WIDE = new Set(["u64", "u128"]);
const MAX_DECIMAL = {
  u8: "255",
  u16: "65535",
  u32: "4294967295",
  u64: "18446744073709551615",
  u128: "340282366920938463463374607431768211455",
};

export function transformNumericFunction(body, parameters, returnType, service) {
  const context = new NumericContext(service);
  for (const parameter of parameters) context.env.set(parameter.name, parameter.type);
  return context.transformBlock(body, returnType);
}

export function transformNumericFunctionDeclaration(statement, service) {
  if (!ts.isFunctionDeclaration(statement) || !statement.name || !statement.body) return statement;
  const context = new NumericContext(service);
  const parameters = [];
  const nextParameters = statement.parameters.map((parameter) => {
    const type = typeFromTypeNode(parameter.type);
    if (parameter.name && ts.isIdentifier(parameter.name)) context.env.set(parameter.name.text, type ?? "number");
    parameters.push({ name: parameter.name.getText(), type: type ?? "number" });
    return ts.factory.updateParameterDeclaration(
      parameter,
      parameter.modifiers,
      parameter.dotDotDotToken,
      parameter.name,
      parameter.questionToken,
      runtimeTypeNode(type, parameter.type),
      parameter.initializer,
    );
  });
  const returnType = typeFromTypeNode(statement.type);
  const body = context.transformBlock(statement.body, returnType ?? "number");
  return ts.factory.updateFunctionDeclaration(
    statement,
    statement.modifiers,
    statement.asteriskToken,
    statement.name,
    statement.typeParameters,
    nextParameters,
    runtimeTypeNode(returnType, statement.type),
    body,
  );
}

class NumericContext {
  constructor(service) {
    this.service = service;
    this.env = new Map();
    this.states = new Map();
    this.helpers = new Map();
    for (const state of service.states ?? []) {
      this.states.set(state.name, {
        key: state.kind === "Scalar" ? "unit" : fromIrType(state.key_type),
        value: fromIrType(state.value_type),
      });
    }
    for (const helper of service.helpers ?? []) {
      this.helpers.set(helper.name, {
        parameters: helper.parameters.map((parameter) => fromIrType(parameter.type)),
        returnType: fromIrType(helper.return_type),
      });
    }
  }

  transformBlock(block, returnType) {
    const statements = block.statements.map((statement) => this.transformStatement(statement, returnType));
    return ts.factory.updateBlock(block, statements);
  }

  transformStatement(statement, returnType) {
    if (ts.isVariableStatement(statement)) {
      const declarations = statement.declarationList.declarations.map((declaration) => {
        if (!ts.isIdentifier(declaration.name)) return declaration;
        const explicit = typeFromTypeNode(declaration.type);
        const expected = explicit ?? this.infer(declaration.initializer);
        const initializer = declaration.initializer
          ? this.transformExpression(declaration.initializer, expected)
          : declaration.initializer;
        const actual = explicit ?? this.infer(declaration.initializer) ?? "number";
        this.env.set(declaration.name.text, actual);
        return ts.factory.updateVariableDeclaration(
          declaration,
          declaration.name,
          undefined,
          undefined,
          initializer,
        );
      });
      return ts.factory.updateVariableStatement(statement, statement.modifiers, ts.factory.updateVariableDeclarationList(
        statement.declarationList,
        declarations,
      ));
    }
    if (ts.isReturnStatement(statement)) {
      return ts.factory.updateReturnStatement(
        statement,
        statement.expression ? this.transformExpression(statement.expression, returnType) : statement.expression,
      );
    }
    if (ts.isIfStatement(statement)) {
      return ts.factory.updateIfStatement(
        statement,
        this.transformExpression(statement.expression),
        this.transformStatement(statement.thenStatement, returnType),
        statement.elseStatement ? this.transformStatement(statement.elseStatement, returnType) : undefined,
      );
    }
    if (ts.isBlock(statement)) return this.transformBlock(statement, returnType);
    if (ts.isExpressionStatement(statement)) {
      return ts.factory.updateExpressionStatement(statement, this.transformExpression(statement.expression));
    }
    if (ts.isWhileStatement(statement)) {
      return ts.factory.updateWhileStatement(statement, this.transformExpression(statement.expression), this.transformStatement(statement.statement, returnType));
    }
    if (ts.isDoStatement(statement)) {
      return ts.factory.updateDoStatement(statement, this.transformStatement(statement.statement, returnType), this.transformExpression(statement.expression));
    }
    if (ts.isForStatement(statement)) {
      return ts.factory.updateForStatement(
        statement,
        statement.initializer && ts.isExpression(statement.initializer)
          ? this.transformExpression(statement.initializer)
          : statement.initializer,
        statement.condition ? this.transformExpression(statement.condition) : undefined,
        statement.incrementor ? this.transformExpression(statement.incrementor) : undefined,
        this.transformStatement(statement.statement, returnType),
      );
    }
    return statement;
  }

  transformExpression(node, expected) {
    if (!node) return node;
    if (ts.isAsExpression(node)) return this.transformAsExpression(node, expected);
    if (isFixed(expected)) assertSameNumericType(expected, this.infer(node), node, node);
    if (ts.isParenthesizedExpression(node)) {
      return ts.factory.updateParenthesizedExpression(node, this.transformExpression(node.expression, expected));
    }
    if (node.kind === ts.SyntaxKind.BigIntLiteral) {
      const type = expected && isFixed(expected) ? expected : this.infer(node);
      if (!isFixed(type)) throw new Error("JAM1200: bigint literal requires a fixed-width integer context");
      const decimal = node.getText().replace(/n$/, "");
      assertDecimalLiteral(decimal, type);
      return isWide(type)
        ? ts.factory.createCallExpression(ts.factory.createIdentifier(constantName(type)), undefined, [ts.factory.createStringLiteral(decimal)])
        : ts.factory.createNumericLiteral(decimal);
    }
    if (ts.isNumericLiteral(node) && expected && isFixed(expected)) {
      const decimal = node.getText();
      assertDecimalLiteral(decimal, expected);
      return isWide(expected)
        ? ts.factory.createCallExpression(ts.factory.createIdentifier(constantName(expected)), undefined, [ts.factory.createStringLiteral(decimal)])
        : ts.factory.createNumericLiteral(decimal);
    }
    if (ts.isBinaryExpression(node)) return this.transformBinary(node, expected);
    if (ts.isPrefixUnaryExpression(node) || ts.isPostfixUnaryExpression(node)) {
      const type = this.infer(node.operand);
      if (isFixed(type) && (node.operator === ts.SyntaxKind.PlusPlusToken || node.operator === ts.SyntaxKind.MinusMinusToken || node.operator === ts.SyntaxKind.PlusToken || node.operator === ts.SyntaxKind.MinusToken)) {
        throw new Error("JAM1207: increment, decrement, and unary arithmetic on fixed-width values must use checked operators");
      }
      const operand = this.transformExpression(node.operand, type);
      return ts.isPrefixUnaryExpression(node)
        ? ts.factory.updatePrefixUnaryExpression(node, operand)
        : ts.factory.updatePostfixUnaryExpression(node, operand);
    }
    if (ts.isCallExpression(node)) return this.transformCall(node, expected);
    if (ts.isPropertyAccessExpression(node)) {
      return ts.factory.updatePropertyAccessExpression(node, this.transformExpression(node.expression), node.name);
    }
    if (ts.isElementAccessExpression(node)) {
      return ts.factory.updateElementAccessExpression(node, this.transformExpression(node.expression), node.argumentExpression ? this.transformExpression(node.argumentExpression) : undefined);
    }
    if (ts.isObjectLiteralExpression(node)) {
      const fields = expected && expected.kind === "record" ? new Map(expected.fields.map((field) => [field.name, field.type])) : new Map();
      return ts.factory.updateObjectLiteralExpression(node, node.properties.map((property) => {
        if (!ts.isPropertyAssignment(property)) return property;
        const name = property.name?.getText();
        return ts.factory.updatePropertyAssignment(property, property.name, this.transformExpression(property.initializer, fields.get(name)));
      }));
    }
    if (ts.isArrowFunction(node) || ts.isFunctionExpression(node)) {
      const nested = new NumericContext(this.service);
      for (const parameter of node.parameters) nested.env.set(parameter.name.getText(), typeFromTypeNode(parameter.type) ?? "number");
      const body = ts.isBlock(node.body) ? nested.transformBlock(node.body, typeFromTypeNode(node.type) ?? "number") : nested.transformExpression(node.body, typeFromTypeNode(node.type));
      return ts.isArrowFunction(node)
        ? ts.factory.updateArrowFunction(node, node.modifiers, node.typeParameters, node.parameters, node.type ? runtimeTypeNode(typeFromTypeNode(node.type), node.type) : undefined, node.equalsGreaterThanToken, body)
        : ts.factory.updateFunctionExpression(node, node.modifiers, node.asteriskToken, node.name, node.typeParameters, node.parameters, node.type ? runtimeTypeNode(typeFromTypeNode(node.type), node.type) : undefined, body);
    }
    if (ts.isConditionalExpression(node)) {
      return ts.factory.updateConditionalExpression(
        node,
        this.transformExpression(node.condition),
        node.questionToken,
        this.transformExpression(node.whenTrue, expected),
        node.colonToken,
        this.transformExpression(node.whenFalse, expected),
      );
    }
    if (ts.isNonNullExpression(node)) return ts.factory.updateNonNullExpression(node, this.transformExpression(node.expression, expected));
    return node;
  }

  transformAsExpression(node, expected) {
    const source = this.infer(node.expression);
    const target = typeFromTypeNode(node.type);
    if (isFixed(source) || isFixed(target)) {
      if (isFixed(source) && isFixed(target) && source === target) {
        return this.transformExpression(node.expression, expected ?? target);
      }
      throw new Error(`JAM1208: numeric type assertions are not casts near ${node.getText()}`);
    }
    return ts.factory.updateAsExpression(node, this.transformExpression(node.expression, expected), node.type);
  }

  transformBinary(node, expected) {
    const operator = node.operatorToken.kind;
    const inferredLeft = this.infer(node.left);
    const inferredRight = this.infer(node.right);
    const leftType = inferredLeft ?? (isContextualLiteral(node.left) ? expected : undefined);
    const rightType = inferredRight ?? (isContextualLiteral(node.right) ? (leftType ?? expected) : undefined);
    if (operator === ts.SyntaxKind.QuestionQuestionToken) {
      const type = isFixed(leftType) ? leftType : rightType;
      if (isWide(type)) {
        return ts.factory.createCallExpression(
          ts.factory.createIdentifier(`${runtimePrefix(type)}Or`),
          undefined,
          [this.transformExpression(node.left), this.transformExpression(node.right, type)],
        );
      }
      return ts.factory.updateBinaryExpression(node, this.transformExpression(node.left, type), node.operatorToken, this.transformExpression(node.right, type));
    }
    if (operator === ts.SyntaxKind.EqualsToken) {
      return ts.factory.updateBinaryExpression(node, this.transformExpression(node.left), node.operatorToken, this.transformExpression(node.right, leftType));
    }
    const compound = new Map([
      [ts.SyntaxKind.PlusEqualsToken, "Add"],
      [ts.SyntaxKind.MinusEqualsToken, "Sub"],
      [ts.SyntaxKind.AsteriskEqualsToken, "Mul"],
    ]).get(operator);
    if (compound) {
      if (!isFixed(leftType)) return ts.factory.updateBinaryExpression(node, this.transformExpression(node.left), node.operatorToken, this.transformExpression(node.right));
      assertSameNumericType(leftType, rightType, node, node.right);
      return ts.factory.updateBinaryExpression(
        node,
        this.transformExpression(node.left, leftType),
        ts.factory.createToken(ts.SyntaxKind.EqualsToken),
        this.numericCall(leftType, compound, node.left, node.right),
      );
    }
    const operation = new Map([
      [ts.SyntaxKind.PlusToken, "Add"],
      [ts.SyntaxKind.MinusToken, "Sub"],
      [ts.SyntaxKind.AsteriskToken, "Mul"],
      [ts.SyntaxKind.SlashToken, "Div"],
      [ts.SyntaxKind.PercentToken, "Mod"],
    ]).get(operator);
    const comparison = new Map([
      [ts.SyntaxKind.LessThanToken, -1],
      [ts.SyntaxKind.LessThanEqualsToken, -2],
      [ts.SyntaxKind.GreaterThanToken, 1],
      [ts.SyntaxKind.GreaterThanEqualsToken, 2],
      [ts.SyntaxKind.EqualsEqualsToken, 0],
      [ts.SyntaxKind.EqualsEqualsEqualsToken, 0],
      [ts.SyntaxKind.ExclamationEqualsToken, 3],
      [ts.SyntaxKind.ExclamationEqualsEqualsToken, 3],
    ]).get(operator);
    if (operation || comparison !== undefined) {
      const type = leftType ?? rightType;
      if (!isFixed(type)) return ts.factory.updateBinaryExpression(node, this.transformExpression(node.left), node.operatorToken, this.transformExpression(node.right));
      assertSameNumericType(type, leftType, node, node.left);
      assertSameNumericType(type, rightType, node, node.right);
      if (operation) return this.numericCall(type, operation, node.left, node.right);
      const compare = ts.factory.createCallExpression(
        ts.factory.createIdentifier(compareName(type)),
        undefined,
        [this.transformExpression(node.left, type), this.transformExpression(node.right, type)],
      );
      if (comparison === 0) return ts.factory.createBinaryExpression(compare, ts.factory.createToken(ts.SyntaxKind.EqualsEqualsEqualsToken), ts.factory.createNumericLiteral(0));
      if (comparison === 3) return ts.factory.createBinaryExpression(compare, ts.factory.createToken(ts.SyntaxKind.ExclamationEqualsEqualsToken), ts.factory.createNumericLiteral(0));
      const token = comparison < 0
        ? (comparison === -1 ? ts.SyntaxKind.LessThanToken : ts.SyntaxKind.LessThanEqualsToken)
        : (comparison === 1 ? ts.SyntaxKind.GreaterThanToken : ts.SyntaxKind.GreaterThanEqualsToken);
      return ts.factory.createBinaryExpression(compare, ts.factory.createToken(token), ts.factory.createNumericLiteral(0));
    }
    return ts.factory.updateBinaryExpression(node, this.transformExpression(node.left), node.operatorToken, this.transformExpression(node.right));
  }

  numericCall(type, operation, left, right) {
    return ts.factory.createCallExpression(
      ts.factory.createIdentifier(`${runtimePrefix(type)}${operation}Checked`),
      undefined,
      [this.transformExpression(left, type), this.transformExpression(right, type)],
    );
  }

  transformCall(node, expected) {
    if (ts.isIdentifier(node.expression) && /^toU(8|16|32|64|128)$/.test(node.expression.text)) {
      if (node.arguments.length !== 1) throw new Error(`JAM1206: ${node.expression.text} expects one argument`);
      const target = node.expression.text.toLowerCase().slice(2);
      const source = this.infer(node.arguments[0]);
      const helper = castName(source, target);
      if (helper === undefined) throw new Error(`JAM1206: cannot checked-cast ${source ?? "unknown"} to ${target}`);
      const value = this.transformExpression(node.arguments[0], source);
      return helper === null
        ? value
        : ts.factory.createCallExpression(ts.factory.createIdentifier(helper), undefined, [value]);
    }
    if (ts.isPropertyAccessExpression(node.expression) && ts.isIdentifier(node.expression.expression)) {
      const state = this.states.get(node.expression.expression.text);
      if (state) {
        const method = node.expression.name.text;
        if (method === "get" || method === "has" || method === "delete") {
          if (node.arguments.length !== (state.key === "unit" ? 0 : 1)) throw new Error(`JAM1201: invalid ${method} arguments for state ${node.expression.expression.text}`);
          const args = node.arguments.map((argument) => this.transformExpression(argument, state.key));
          return ts.factory.updateCallExpression(node, node.expression, node.typeArguments, args);
        }
        if (method === "set") {
          const expectedArgs = state.key === "unit" ? [state.value] : [state.key, state.value];
          if (node.arguments.length !== expectedArgs.length) throw new Error(`JAM1201: invalid set arguments for state ${node.expression.expression.text}`);
          const args = node.arguments.map((argument, index) => this.transformExpression(argument, expectedArgs[index]));
          return ts.factory.updateCallExpression(node, node.expression, node.typeArguments, args);
        }
      }
    }
    const helper = ts.isIdentifier(node.expression) ? this.helpers.get(node.expression.text) : undefined;
    const args = node.arguments.map((argument, index) => this.transformExpression(argument, helper?.parameters[index]));
    return ts.factory.updateCallExpression(node, this.transformExpression(node.expression), node.typeArguments, args);
  }

  infer(node) {
    if (!node) return undefined;
    if (ts.isParenthesizedExpression(node)) return this.infer(node.expression);
    if (ts.isIdentifier(node)) return this.env.get(node.text);
    if (node.kind === ts.SyntaxKind.BigIntLiteral) return undefined;
    if (ts.isNumericLiteral(node)) return "number";
    if (ts.isPropertyAccessExpression(node)) {
      const object = this.infer(node.expression);
      if (object?.kind === "record") return object.fields.find((field) => field.name === node.name.text)?.type;
      if (ts.isIdentifier(node.expression)) return this.states.get(node.expression.text)?.value;
      return undefined;
    }
    if (ts.isCallExpression(node)) {
      if (ts.isIdentifier(node.expression) && /^toU(8|16|32|64|128)$/.test(node.expression.text)) return node.expression.text.toLowerCase().slice(2);
      if (ts.isPropertyAccessExpression(node.expression) && ts.isIdentifier(node.expression.expression)) {
        const state = this.states.get(node.expression.expression.text);
        if (state && node.expression.name.text === "get") return state.value;
      }
      if (ts.isIdentifier(node.expression)) return this.helpers.get(node.expression.text)?.returnType;
      return undefined;
    }
    if (ts.isBinaryExpression(node)) {
      if (node.operatorToken.kind === ts.SyntaxKind.QuestionQuestionToken) return this.infer(node.left) ?? this.infer(node.right);
      return this.infer(node.left) ?? this.infer(node.right);
    }
    if (ts.isAsExpression(node)) return this.infer(node.expression);
    return undefined;
  }

  createTransformationContext() {
    return { factory: ts.factory };
  }
}

function fromIrType(type) {
  if (typeof type === "string") {
    const map = { U8: "u8", U16: "u16", U32: "u32", U64: "u64", U128: "u128", Bool: "bool", Address: "bytes", Unit: "unit" };
    return map[type] ?? type.toLowerCase();
  }
  const kind = Object.keys(type)[0];
  const data = type[kind];
  if (kind === "Record") return { kind: "record", fields: data.fields.map((field) => ({ name: field.name, type: fromIrType(field.ty) })) };
  if (kind === "FixedBytes") return "bytes";
  return kind.toLowerCase();
}

function typeFromTypeNode(node) {
  if (node?.kind === ts.SyntaxKind.NumberKeyword) return "number";
  if (!node || !ts.isTypeReferenceNode(node) || !ts.isIdentifier(node.typeName)) return undefined;
  const name = node.typeName.text.toLowerCase();
  return FIXED.has(name) ? name : undefined;
}

function runtimeTypeNode(type, original) {
  if (!type || !isFixed(type)) return original && !isFixed(typeFromTypeNode(original)) ? original : undefined;
  return ts.factory.createTypeReferenceNode(type === "u64" ? "JamU64" : type === "u128" ? "JamU128" : "number", undefined);
}

function isFixed(type) { return typeof type === "string" && FIXED.has(type); }
function isWide(type) { return typeof type === "string" && WIDE.has(type); }
function isNumeric(type) { return isFixed(type); }
function isContextualLiteral(node) {
  if (ts.isParenthesizedExpression(node)) return isContextualLiteral(node.expression);
  return ts.isNumericLiteral(node) || node.kind === ts.SyntaxKind.BigIntLiteral;
}
function constantName(type) { return `${runtimePrefix(type)}Const`; }
function compareName(type) { return `${runtimePrefix(type)}Compare`; }
function runtimePrefix(type) { return `jam${type[0].toUpperCase()}${type.slice(1)}`; }

function castName(source, target) {
  if (!source || !isFixed(target)) return undefined;
  if (source === target) return null;
  if (source === "number" || ["u8", "u16", "u32"].includes(source)) return `jam${target[0].toUpperCase()}${target.slice(1)}FromNumber`;
  if (source === "u64" && target === "u128") return "jamU128FromU64";
  if (source === "u128" && target === "u64") return "jamU64FromU128";
  if (source === "u64" && ["u8", "u16", "u32"].includes(target)) return `jam${target[0].toUpperCase()}${target.slice(1)}FromU64`;
  if (source === "u128" && ["u8", "u16", "u32"].includes(target)) return `jam${target[0].toUpperCase()}${target.slice(1)}FromU128`;
  return undefined;
}

function assertSameNumericType(expected, actual, node, operand) {
  if (!isFixed(expected) || actual === expected) return;
  if ((actual === "number" || actual === undefined) && isContextualLiteral(operand)) return;
  throw new Error(`JAM1203: fixed-width numeric operands must have identical types near ${node.getText()}`);
}

function assertDecimalLiteral(decimal, type) {
  if (!/^\d+$/.test(decimal)) throw new Error(`JAM1202: ${type} literal must be a decimal integer`);
  const normalized = decimal.replace(/^0+(?=\d)/, "");
  const max = MAX_DECIMAL[type];
  if (normalized.length > max.length || (normalized.length === max.length && normalized > max)) throw new Error(`JAM1204: ${type} literal is out of range`);
}
