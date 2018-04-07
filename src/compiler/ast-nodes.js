import { nodeTypes } from './constants';

function assertParts(nodeType, nodeParts, expectedPartNames) {
  expectedPartNames.forEach(part => {
    if (typeof nodeParts[part] === 'undefined') {
      throw new Error(`Missing part ${part} on node of type ${nodeType}`);
    }
  });
}

const nodeTypesToParts = {
  Array: { type: nodeTypes.ARRAY, parts: ['elements'] },

  Assignment: { type: nodeTypes.ASSIGNMENT, parts: ['leftSide', 'rightSide'] },

  Boolean: { type: nodeTypes.BOOLEAN, parts: ['value'] },

  Call: { type: nodeTypes.CALL, parts: ['callee', 'arg'] },

  Conditional: {
    type: nodeTypes.CONDITIONAL,
    parts: ['predicate', 'thenCase', 'elseCase']
  },

  Function: { type: nodeTypes.FUNCTION, parts: ['parameter', 'body'] },

  FunctionType: { type: nodeTypes.FUNCTION_TYPE, parts: ['from', 'to'] },

  Identifier: {
    type: nodeTypes.IDENTIFIER,
    parts: ['value', 'isGetter', 'isSetter']
  },

  InterpolatedString: {
    type: nodeTypes.INTERPOLATED_STRING,
    parts: ['literals', 'expressions']
  },

  MemberExpression: {
    type: nodeTypes.MEMBER_EXPRESSION,
    parts: ['parts']
  },

  Module: { type: nodeTypes.MODULE, parts: ['body'] },

  Number: { type: nodeTypes.NUMBER, parts: ['value'] },

  Object: { type: nodeTypes.OBJECT, parts: ['properties'] },

  ObjectProperty: { type: nodeTypes.OBJECT_PROPERTY, parts: ['key', 'value'] },

  String: { type: nodeTypes.STRING, parts: ['value'] },

  Tuple: { type: nodeTypes.TUPLE, parts: ['entries'] },

  TypeAliasDeclaration: {
    type: nodeTypes.TYPE_ALIAS_DECLARATION,
    parts: ['typeName', 'typeParameters', 'typeExpression']
  },

  TypeConstructor: {
    type: nodeTypes.TYPE_CONSTRUCTOR,
    parts: ['typeName', 'typeParameters']
  },

  TypeDeclaration: {
    type: nodeTypes.TYPE_DECLARATION,
    parts: ['typeName', 'typeParameters', 'typeConstructors']
  },

  RecordType: {
    type: nodeTypes.RECORD_TYPE,
    parts: ['entries']
  },

  RecordTypeEntry: {
    type: nodeTypes.RECORD_TYPE_ENTRY,
    parts: ['name', 'typeExpression']
  },

  TypeTag: {
    type: nodeTypes.TYPE_TAG,
    parts: ['typeTagName', 'typeExpression']
  },

  TypeTuple: { type: nodeTypes.TYPE_TUPLE, parts: ['typeEntries'] },

  TypeVariable: { type: nodeTypes.TYPE_VARIABLE, parts: ['typeName'] }
};

export const buildNode = Object.keys(nodeTypesToParts).reduce(
  (builders, type) => {
    builders[type] = (lineStart, lineEnd) => nodeParts => {
      const nodeType = nodeTypesToParts[type].type;
      const expectedParts = nodeTypesToParts[type].parts;

      if (nodeType === undefined) {
        throw new Error(`nodeType is undefined for node of type ${type}`);
      }

      expectedParts.forEach(part => {
        if (nodeParts[part] === undefined) {
          throw new Error(`Part ${part} is not given for node of type ${type}`);
        }
      });

      return {
        type: nodeType,
        lineStart,
        lineEnd,
        ...nodeParts
      };
    };

    return builders;
  },
  {}
);
