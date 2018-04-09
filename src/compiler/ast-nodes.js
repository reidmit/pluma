const nodes = {
  Array: {
    props: ['elements']
  },

  Assignment: {
    props: ['id', 'typeAnnotation', 'value']
  },

  Boolean: {
    props: ['value']
  },

  Call: {
    props: ['callee', 'argument']
  },

  Conditional: {
    props: ['predicate', 'thenCase', 'elseCase']
  },

  Function: {
    props: ['parameter', 'body']
  },

  FunctionType: {
    props: ['from', 'to']
  },

  Identifier: {
    props: ['value', 'isGetter', 'isSetter']
  },

  InterpolatedString: {
    props: ['literals', 'expressions']
  },

  LetExpression: {
    props: ['assignments', 'body']
  },

  MemberExpression: {
    props: ['parts']
  },

  Module: {
    props: ['body']
  },

  Number: {
    props: ['value']
  },

  PipeExpression: {
    props: ['left', 'right']
  },

  Record: {
    props: ['properties']
  },

  RecordProperty: {
    props: ['key', 'value']
  },

  RecordPropertyType: {
    props: ['key', 'value']
  },

  RecordType: {
    props: ['properties']
  },

  String: {
    props: ['value']
  },

  Tuple: {
    props: ['entries']
  },

  TupleType: {
    props: ['typeEntries']
  },

  TypeAliasDeclaration: {
    props: ['typeName', 'typeParameters', 'typeExpression']
  },

  TypeConstructor: {
    props: ['typeName', 'typeParameters']
  },

  TypeDeclaration: {
    props: ['typeName', 'typeParameters', 'typeConstructors']
  },

  TypeTag: {
    props: ['typeTagName', 'typeExpression']
  },

  TypeVariable: {
    props: ['typeName']
  }
};

export const buildNode = Object.keys(nodes).reduce((builders, kind) => {
  builders[kind] = (lineStart, lineEnd) => nodeProps => {
    const expectedProps = nodes[kind].props;

    expectedProps.forEach(prop => {
      if (nodeProps[prop] === undefined) {
        throw new Error(
          `Property ${prop} is not given for node of kind ${kind}`
        );
      }
    });

    return {
      kind,
      lineStart,
      lineEnd,
      ...nodeProps
    };
  };

  return builders;
}, {});
