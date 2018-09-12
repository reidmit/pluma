// https://brianmckenna.org/files/js-type-inference/docs/typeinference.html

/// BEGIN TYPES

function TypeVariable() {
  this.id = TypeVariable.nextId;
  TypeVariable.nextId++;
  this.instance = null;
}
TypeVariable.nextId = 0;
TypeVariable.prototype.toString = function() {
  if (!this.instance) {
    return `t${this.id}`;
  }

  return this.instance.toString();
};

function BaseType(name, types) {
  this.name = name;
  this.types = types;
}
BaseType.prototype.toString = function() {
  if (this.types && this.types.length) {
    return this.types.map(type => type.toString()).join(' -> ');
  } else if (this.elementType) {
    return `[${this.elementType.toString()}]`;
  } else if (this.rowTypes) {
    return (
      '{ ' +
      this.rowTypes
        .map(rowType => `${rowType.propertyName} :: ${rowType.propertyType}`)
        .join(', ') +
      ' }'
    );
  }
  return this.name;
};

const FunctionType = function(types) {
  this.types = types;
};
FunctionType.prototype = new BaseType('Function');

const NumberType = function() {};
NumberType.prototype = new BaseType('Number', []);

const StringType = function() {};
StringType.prototype = new BaseType('String', []);

const ArrayType = function(elementType) {
  this.elementType = elementType;
};
ArrayType.prototype = new BaseType('Array');

const RowType = function(name, type) {
  this.propertyName = name;
  this.propertyType = type;
};
RowType.prototype = new BaseType('Row');

const ObjectType = function(rowTypes) {
  this.rowTypes = rowTypes;
};
ObjectType.prototype = new BaseType('Object');

/// END TYPES

const prune = type => {
  if (type instanceof TypeVariable && type.instance) {
    type.instance = prune(type.instance);
    return type.instance;
  }

  return type;
};

const fresh = (type, nonGeneric, mappings = {}) => {
  type = prune(type);
  if (type instanceof TypeVariable) {
    if (occursInTypeArray(type, nonGeneric)) {
      return type;
    } else {
      if (!mappings[type.id]) {
        mappings[type.id] = new TypeVariable();
      }
      return mappings[type.id];
    }
  }

  if (type.propertyName && type.propertyType) {
    return new RowType(type.propertyName, type.propertyType);
  }

  if (type.rowTypes) {
    return new ObjectType(type.rowTypes.map(t => fresh(t, nonGeneric, mappings)));
  }

  return new BaseType(type.name, type.types.map(t => fresh(t, nonGeneric, mappings)));
};

const occursInType = (type1, type2) => {
  type2 = prune(type2);
  if (type2 === type1) return true;
  if (type2 instanceof BaseType) return occursInTypeArray(type1, type2.types);
  return false;
};

const occursInTypeArray = (type1, types) => {
  for (let i = 0; i < types.length; i++) {
    if (occursInType(type1, types[i])) return true;
  }
  return false;
};

const unify = (type1, type2) => {
  type1 = prune(type1);
  type2 = prune(type2);

  if (type1 instanceof TypeVariable) {
    if (type1 !== type2) {
      if (occursInType(type1, type2)) {
        throw 'recursive unification??';
      }

      type1.instance = type2;
    }

    return;
  }

  if (type1 instanceof BaseType && type2 instanceof TypeVariable) {
    unify(type2, type1);
    return;
  }

  if (type1 instanceof BaseType && type2 instanceof BaseType) {
    if (type1.name !== type2.name || type1.types.length !== type2.types.length) {
      throw `Type error! ${type1} is not ${type2}`;
    }

    for (let i = 0; i < Math.min(type1.types.length, type2.types.length); i++) {
      unify(type1.types[i], type2.types[i]);
    }

    return;
  }

  throw 'Not unified????';
};

const analyze = (node, env, nonGeneric = []) => {
  if (node.type === 'ExpressionStatement') {
    return analyze(node.expression, env, nonGeneric);
  }

  if (node.type === 'NumericLiteral') {
    return new NumberType();
  }

  if (node.type === 'StringLiteral') {
    return new StringType();
  }

  if (node.type === 'Identifier') {
    if (!env[node.name]) {
      throw `"${node.name}" is not defined`;
    }

    return fresh(env[node.name], nonGeneric);
  }

  if (node.type === 'VariableDeclaration') {
    return analyze(node.declarations[0], env, nonGeneric);
  }

  if (node.type === 'VariableDeclarator') {
    // Basic case, assigning a single variable
    if (node.id.type === 'Identifier') {
      const valueType = analyze(node.init, env, nonGeneric);

      // TODO: if we have type annotation for this,
      // unify it to make sure it matches valueType

      env[node.id.name] = valueType;
      return valueType;
    }
  }

  if (node.type === 'ArrowFunctionExpression') {
    // A function type consists of a list of types:
    // one for each parameter, and then finally its return type

    const funcTypes = [];
    const newNonGeneric = nonGeneric.slice();
    node.params.forEach(param => {
      // TODO: check if we have type info for this param
      // and if so, use that

      const paramType = new TypeVariable();
      newNonGeneric.push(paramType);
      env[param.name] = paramType;
      funcTypes.push(paramType);
    });

    const returnType = analyze(node.body, env, newNonGeneric);
    funcTypes.push(returnType);

    // TODO: check if we have type info for this func,
    // and if so, unify it with what we have collected to
    // ensure correctness

    return new FunctionType(funcTypes);
  }

  if (node.type === 'CallExpression') {
    const funcTypes = [];
    node.arguments.forEach(arg => {
      const argType = analyze(arg, env, nonGeneric);
      funcTypes.push(argType);
    });

    const returnType = new TypeVariable();
    funcTypes.push(returnType);

    const apparentType = new FunctionType(funcTypes);
    const actualType = analyze(node.callee, env, nonGeneric);
    unify(apparentType, actualType);

    return returnType;
  }

  if (node.type === 'ArrayExpression') {
    let elementType;
    node.elements.forEach(element => {
      const currentElementType = analyze(element, env, nonGeneric);
      if (!elementType) {
        elementType = currentElementType;
      } else {
        unify(currentElementType, elementType);
      }
    });

    return new ArrayType(elementType);
  }

  if (node.type === 'ObjectExpression') {
    const rowTypes = [];
    node.properties.forEach(property => {
      const propertyName = property.key.name;
      const propertyType = analyze(property.value, env, nonGeneric);
      rowTypes.push(new RowType(propertyName, propertyType));
    });

    return new ObjectType(rowTypes);
  }

  if (node.type === 'MemberExpression') {
    console.log({ node });
    const objectType = analyze(node.object, env, nonGeneric);
    console.log({ objectType });
  }

  console.log(node.type);
};

const checkTypes = ({ ast }) => {
  const body = ast.program.body;

  const env = {};

  body.forEach(node => {
    const type = analyze(node, env);
    console.log({ node, type: type.toString() });
  });

  return null;
};

export default checkTypes;
