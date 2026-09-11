from rest_framework import serializers


class UserSerializer(serializers.Serializer):
    id = serializers.IntegerField(read_only=True)
    name = serializers.CharField(max_length=80)
    email = serializers.EmailField(required=False)


class TagSerializer(serializers.Serializer):
    slug = serializers.SlugField()
    label = serializers.CharField()
