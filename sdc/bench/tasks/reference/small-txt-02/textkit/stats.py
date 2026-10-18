from collections import Counter

from .tokenize import words


def word_freq(text):
    return Counter(words(text))


def top_n(text, n):
    """The n most frequent words as (word, count), most frequent first; ties alphabetical."""
    counts = word_freq(text)
    return sorted(counts.items(), key=lambda pair: (-pair[1], pair[0]))[:n]


def average_word_length(text):
    found = words(text)
    if not found:
        return 0.0
    return sum(len(word) for word in found) / len(found)
