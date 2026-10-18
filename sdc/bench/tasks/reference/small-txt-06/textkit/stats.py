from collections import Counter

from .tokenize import split_words


def word_freq(text):
    return Counter(split_words(text))


def top_n(text, n):
    """The n most frequent words as (word, count), most frequent first; ties alphabetical."""
    counts = word_freq(text)
    return sorted(counts.items(), key=lambda pair: (-pair[1], pair[0]))[:n]


def average_word_length(text):
    found = split_words(text)
    return sum(len(word) for word in found) / len(found)
