"""Parse declared scheduling operands from a retained annotation."""

import re


SCHEDULE_MODES = {
    "Fixed delay": ("fixedDelay", "fixedDelayString"),
    "Fixed rate": ("fixedRate", "fixedRateString"),
    "Cron": ("cron",),
}
DECLARATION_ANNOTATION = re.compile(r"\s*@((?:[\w$]+\.)*[\w$]+)(?:\s*(\())?")
SCHEDULE_TOKEN = re.compile(r'"(?:[^"\\\r\n]|\\.)*"|\'(?:[^\'\\\r\n]|\\.)*\'|//[^\r\n]*|/\*[\s\S]*?\*/|[(),\[\]{}]')


def annotation_arguments(text, start):
    arguments = []
    stack = ["("]
    for token in SCHEDULE_TOKEN.finditer(text, start):
        value = token[0]
        if value in {"(", "[", "{"}:
            stack.append(value)
        elif value in {")", "]", "}"}:
            if stack.pop() != {")": "(", "]": "[", "}": "{"}[value]:
                break
            if not stack:
                arguments.append(text[start:token.start()])
                return arguments, token.end()
        elif value == "," and len(stack) == 1:
            arguments.append(text[start:token.start()])
            start = token.end()
    return [], len(text)


def scheduled_arguments(text):
    matches = []
    offset = 0
    while annotation := DECLARATION_ANNOTATION.match(text, offset):
        offset = annotation.end()
        arguments = []
        if annotation[2]:
            arguments, offset = annotation_arguments(text, offset)
        if annotation[1] in {"Scheduled", "org.springframework.scheduling.annotation.Scheduled"}:
            matches.append(arguments)
    if len(matches) != 1:
        return {}
    pairs = [re.fullmatch(r"\s*(\w+)\s*=\s*(.+?)\s*", argument, re.S)
             for argument in matches[0]]
    return {match[1]: match[2] for match in pairs if match}
